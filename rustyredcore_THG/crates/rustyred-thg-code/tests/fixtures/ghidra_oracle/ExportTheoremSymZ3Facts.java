// Headless Ghidra SymbolicSummaryZ3 exporter for Theorem's program-analysis
// oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-symz3-oracle -import hello_tiny.o \
//   -postScript ExportTheoremSymZ3Facts.java hello_tiny.symz3.oracle.json 256 128
//
// Args:
//   0: output path
//   1: max functions to summarize, default 256
//   2: max instructions per function, default 128

import com.microsoft.z3.BitVecExpr;
import com.microsoft.z3.BoolExpr;
import com.microsoft.z3.Context;
import ghidra.app.script.GhidraScript;
import ghidra.pcode.emu.symz3.SymZ3;
import ghidra.pcode.emu.symz3.SymZ3MemoryMap;
import ghidra.pcode.emu.symz3.SymZ3PcodeExecutorStatePiece;
import ghidra.pcode.emu.symz3.SymZ3PcodeThread;
import ghidra.pcode.emu.symz3.SymZ3RecordsExecution;
import ghidra.pcode.emu.symz3.SymZ3RegisterMap;
import ghidra.pcode.emu.symz3.lib.Z3InfixPrinter;
import ghidra.pcode.emu.symz3.lib.Z3MemoryWitness;
import ghidra.pcode.emu.symz3.state.SymZ3PcodeEmulator;
import ghidra.pcode.exec.PcodeStateCallbacks;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSpace;
import ghidra.program.model.listing.Function;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.pcode.PcodeOp;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.symz3.model.SymValueZ3;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.stream.Collectors;

public class ExportTheoremSymZ3Facts extends GhidraScript {
    private static final int DEFAULT_MAX_FUNCTIONS = 256;
    private static final int DEFAULT_MAX_STEPS = 128;
    private static final int MEMORY_COPY_CHUNK = 4096;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outputPath = args.length > 0
            ? args[0]
            : "theorem-ghidra-symz3-oracle.json";
        int maxFunctions = intArg(args, 1, DEFAULT_MAX_FUNCTIONS);
        int maxSteps = intArg(args, 2, DEFAULT_MAX_STEPS);

        SymZ3.loadZ3Libs();

        List<Function> functions = new ArrayList<>();
        for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
            functions.add(function);
            if (functions.size() >= maxFunctions) {
                break;
            }
        }

        SymbolTable symbols = currentProgram.getSymbolTable();
        int importCount = 0;
        for (ExternalLocation ignored : symbols.getExternalLocations()) {
            importCount++;
        }

        List<String> symbolicSummaries = new ArrayList<>();
        for (Function function : functions) {
            try {
                String summary = summarizeFunction(function, maxSteps);
                if (summary != null) {
                    symbolicSummaries.add(summary);
                }
            }
            catch (Exception e) {
                println("SymZ3 summary skipped for " + function.getName() + " at " +
                    address(function.getEntryPoint()) + ": " + e.getClass().getSimpleName() +
                    ": " + e.getMessage());
            }
        }

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture\": {");
            out.println("    \"fixture_id\": " + json("ghidra:symz3_oracle:" +
                currentProgram.getName()) + ",");
            out.println("    \"source_uri\": " + json(currentProgram.getExecutablePath()) + ",");
            out.println("    \"export_script\": \"ExportTheoremSymZ3Facts.java\",");
            out.println("    \"evidence_ids\": [" +
                json("ghidra:symz3_program:" + currentProgram.getName()) + "],");
            out.println("    \"program_summary\": {");
            out.println("      \"ghidra_version\": " + json(getGhidraVersion()) + ",");
            out.println("      \"language_id\": " +
                json(currentProgram.getLanguageID().toString()) + ",");
            out.println("      \"compiler_spec_id\": " +
                json(currentProgram.getCompilerSpec().getCompilerSpecID().toString()) + ",");
            out.println("      \"analysis_timeout_occurred\": false,");
            out.println("      \"function_count\": " + functions.size() + ",");
            out.println("      \"import_count\": " + importCount + ",");
            out.println("      \"string_count\": 0,");
            out.println("      \"cfg_edge_count\": 0");
            out.println("    }");
            out.println("  },");
            out.println("  \"functions\": [],");
            out.println("  \"pcode_ops\": [],");
            out.println("  \"references\": [],");
            out.println("  \"call_edges\": [],");
            out.println("  \"symbolic_summaries\": [");
            writeObjects(out, symbolicSummaries);
            out.println("  ],");
            out.println("  \"diagnostics\": [");
            writeObjects(out, new ArrayList<>());
            out.println("  ],");
            out.println("  \"semantic_signatures\": [");
            writeObjects(out, new ArrayList<>());
            out.println("  ],");
            out.println("  \"function_id_signatures\": [");
            writeObjects(out, new ArrayList<>());
            out.println("  ]");
            out.println("}");
        }
    }

    private String summarizeFunction(Function function, int maxSteps) throws Exception {
        SymZ3PcodeEmulator emulator = new SymZ3PcodeEmulator(currentProgram.getLanguage());
        copyInitializedMemory(emulator);

        String entry = address(function.getEntryPoint());
        String summaryId = "ghidra:symz3_summary:" + entry;
        String threadId = "theorem-symz3-" + cleanId(function.getName()) + "-" +
            cleanId(entry);

        SymZ3PcodeThread thread = emulator.newThread(threadId);
        thread.overrideCounter(function.getEntryPoint());
        thread.overrideContextWithDefault();
        thread.reInitialize();

        int steps = 0;
        while (steps < maxSteps) {
            Address counter = thread.getCounter();
            if (counter == null || !function.getBody().contains(counter)) {
                break;
            }
            thread.stepInstruction();
            steps++;
        }

        SymZ3PcodeExecutorStatePiece localState = thread.getLocalSymbolicState();
        SymZ3PcodeExecutorStatePiece sharedState = thread.getSharedSymbolicState();
        List<String> preconditions = new ArrayList<>(thread.getPreconditions());
        List<SymZ3RecordsExecution.RecOp> branchOps = branchOps(sharedState);
        List<String> preconditionObjects = preconditionObjects(
            summaryId,
            preconditions,
            branchOps);
        List<String> valueObjects = symbolicValueObjects(summaryId, localState, sharedState);
        List<String> memoryWitnessObjects = memoryWitnessObjects(summaryId, localState, sharedState);
        List<String> registersRead = registerNames(localState, true);
        List<String> registersUpdated = registerNames(localState, false);

        if (preconditionObjects.isEmpty() && valueObjects.isEmpty() &&
            memoryWitnessObjects.isEmpty()) {
            return null;
        }

        List<String> evidenceIds = new ArrayList<>();
        evidenceIds.add("ghidra:symz3_function:" + entry);
        evidenceIds.add("ghidra:function:" + entry);

        return "    {" +
            "\"summary_id\": " + json(summaryId) + ", " +
            "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"thread_id\": " + json(threadId) + ", " +
            "\"preconditions\": [" + String.join(", ", preconditionObjects) + "], " +
            "\"registers_read\": " + jsonArray(registersRead) + ", " +
            "\"registers_updated\": " + jsonArray(registersUpdated) + ", " +
            "\"symbolic_values\": [" + String.join(", ", valueObjects) + "], " +
            "\"memory_witnesses\": [" + String.join(", ", memoryWitnessObjects) + "], " +
            "\"solver_status\": \"not_checked\", " +
            "\"model_bindings\": [], " +
            "\"valuation_hash\": \"\", " +
            "\"evidence_ids\": " + jsonArray(evidenceIds) +
            "}";
    }

    private List<String> preconditionObjects(
            String summaryId,
            List<String> preconditions,
            List<SymZ3RecordsExecution.RecOp> branchOps) {
        List<String> objects = new ArrayList<>();
        try (Context ctx = new Context()) {
            Z3InfixPrinter z3p = new Z3InfixPrinter(ctx);
            for (int index = 0; index < preconditions.size(); index++) {
                String serialized = preconditions.get(index);
                SymZ3RecordsExecution.RecOp branch = index < branchOps.size()
                    ? branchOps.get(index)
                    : null;
                PcodeOp op = branch == null ? null : branch.op();
                String address = op == null || op.getSeqnum() == null
                    ? null
                    : address(op.getSeqnum().getTarget());
                String pcodeId = op == null ? null : pcodeId(op);
                String display = displayBool(ctx, z3p, serialized);
                List<String> evidenceIds = new ArrayList<>();
                evidenceIds.add(summaryId);
                if (pcodeId != null) {
                    evidenceIds.add(pcodeId);
                }
                if (address != null) {
                    evidenceIds.add("ghidra:symz3_branch:" + address);
                }
                objects.add("{" +
                    "\"precondition_id\": " + json(summaryId + ":pre:" + index) + ", " +
                    "\"step_index\": " + index + ", " +
                    "\"address\": " + jsonOrNull(address) + ", " +
                    "\"pcode_op_id\": " + jsonOrNull(pcodeId) + ", " +
                    "\"branch_taken\": " + inferredBranchTaken(serialized) + ", " +
                    "\"serialized_expr\": " + json(serialized) + ", " +
                    "\"display_expr\": " + json(display) + ", " +
                    "\"solver_status\": \"not_checked\", " +
                    "\"model_bindings\": [], " +
                    "\"evidence_ids\": " + jsonArray(evidenceIds) +
                    "}");
            }
        }
        return objects;
    }

    private List<String> symbolicValueObjects(
            String summaryId,
            SymZ3PcodeExecutorStatePiece localState,
            SymZ3PcodeExecutorStatePiece sharedState) {
        List<Map.Entry<String, String>> entries = new ArrayList<>();
        try (Context ctx = new Context()) {
            Z3InfixPrinter z3p = new Z3InfixPrinter(ctx);
            localState.streamValuations(ctx, z3p).forEach(entries::add);
            sharedState.streamValuations(ctx, z3p).forEach(entries::add);
        }
        entries.sort((left, right) -> {
            int byKey = left.getKey().compareTo(right.getKey());
            if (byKey != 0) {
                return byKey;
            }
            return left.getValue().compareTo(right.getValue());
        });

        List<String> objects = new ArrayList<>();
        for (int index = 0; index < entries.size(); index++) {
            Map.Entry<String, String> entry = entries.get(index);
            String name = entry.getKey();
            String expression = entry.getValue();
            objects.add("{" +
                "\"value_id\": " + json(summaryId + ":value:" + index) + ", " +
                "\"kind\": " + json(valueKind(name)) + ", " +
                "\"name\": " + json(name) + ", " +
                "\"space\": " + jsonOrNull(valueSpace(name)) + ", " +
                "\"offset\": null, " +
                "\"byte_len\": " + byteLenOrNull(name) + ", " +
                "\"serialized_expr\": " + json(expression) + ", " +
                "\"display_expr\": " + json(expression) + ", " +
                "\"concrete_value\": null, " +
                "\"evidence_ids\": " + jsonArray(Collections.singletonList(summaryId)) +
                "}");
        }
        return objects;
    }

    private List<String> memoryWitnessObjects(
            String summaryId,
            SymZ3PcodeExecutorStatePiece localState,
            SymZ3PcodeExecutorStatePiece sharedState) throws Exception {
        List<String> objects = new ArrayList<>();
        try (Context ctx = new Context()) {
            Z3InfixPrinter z3p = new Z3InfixPrinter(ctx);
            List<SymZ3MemoryMap> maps = new ArrayList<>();
            maps.addAll(memoryMaps(localState));
            maps.addAll(memoryMaps(sharedState));
            for (SymZ3MemoryMap map : maps) {
                List<Z3MemoryWitness> witnesses = memoryWitnesses(map);
                for (Z3MemoryWitness witness : witnesses) {
                    int index = objects.size();
                    BitVecExpr addressExpr = witness.address().getBitVecExpr(ctx);
                    String addressSerialized = SymValueZ3.serialize(ctx, addressExpr);
                    String addressDisplay = z3p.infixWithBrackets(addressExpr);
                    SymValueZ3 value = witnessValue(map, witness);
                    String valueSerialized = null;
                    String valueDisplay = null;
                    if (value != null) {
                        BitVecExpr valueExpr = value.getBitVecExpr(ctx);
                        valueSerialized = SymValueZ3.serialize(ctx, valueExpr);
                        valueDisplay = z3p.infixUnsigned((BitVecExpr) valueExpr.simplify());
                    }
                    objects.add("{" +
                        "\"witness_id\": " + json(summaryId + ":mem:" + index) + ", " +
                        "\"kind\": " +
                        json(witness.t().toString().toLowerCase(Locale.ROOT)) + ", " +
                        "\"address_expr\": " + json(addressSerialized) + ", " +
                        "\"address_display\": " + json(addressDisplay) + ", " +
                        "\"byte_len\": " + witness.bytesMoved() + ", " +
                        "\"value_expr\": " + jsonOrNull(valueSerialized) + ", " +
                        "\"value_display\": " + jsonOrNull(valueDisplay) + ", " +
                        "\"evidence_ids\": " + jsonArray(Collections.singletonList(summaryId)) +
                        "}");
                }
            }
        }
        return objects;
    }

    private SymValueZ3 witnessValue(SymZ3MemoryMap map, Z3MemoryWitness witness) {
        try {
            return map.load(
                witness.address(),
                witness.bytesMoved(),
                false,
                PcodeStateCallbacks.NONE);
        }
        catch (Exception ignored) {
            return null;
        }
    }

    private void copyInitializedMemory(SymZ3PcodeEmulator emulator) throws Exception {
        for (MemoryBlock block : currentProgram.getMemory().getBlocks()) {
            if (!block.isInitialized()) {
                continue;
            }
            AddressSpace space = block.getStart().getAddressSpace();
            long copied = 0;
            while (copied < block.getSize()) {
                int chunkSize = (int) Math.min(MEMORY_COPY_CHUNK, block.getSize() - copied);
                byte[] bytes = new byte[chunkSize];
                Address chunkStart = block.getStart().add(copied);
                int read = block.getBytes(chunkStart, bytes);
                if (read <= 0) {
                    break;
                }
                if (read != bytes.length) {
                    byte[] trimmed = new byte[read];
                    System.arraycopy(bytes, 0, trimmed, 0, read);
                    bytes = trimmed;
                }
                emulator.getSharedState().getLeft().setVar(
                    space,
                    chunkStart.getOffset(),
                    bytes.length,
                    true,
                    bytes);
                copied += read;
            }
        }
    }

    @SuppressWarnings("unchecked")
    private Map<AddressSpace, ?> spaceMap(SymZ3PcodeExecutorStatePiece state) throws Exception {
        Field field = SymZ3PcodeExecutorStatePiece.class.getDeclaredField("spaceMap");
        field.setAccessible(true);
        return (Map<AddressSpace, ?>) field.get(state);
    }

    private List<String> registerNames(
            SymZ3PcodeExecutorStatePiece state,
            boolean read) throws Exception {
        Set<String> names = new TreeSet<>();
        for (Object space : spaceMap(state).values()) {
            if (!space.getClass().getSimpleName().equals("SymZ3RegisterSpace")) {
                continue;
            }
            Field field = space.getClass().getDeclaredField("rmap");
            field.setAccessible(true);
            SymZ3RegisterMap map = (SymZ3RegisterMap) field.get(space);
            names.addAll(read ? map.getRegisterNamesRead() : map.getRegisterNamesUpdated());
        }
        return new ArrayList<>(names);
    }

    private List<SymZ3MemoryMap> memoryMaps(SymZ3PcodeExecutorStatePiece state) throws Exception {
        List<SymZ3MemoryMap> maps = new ArrayList<>();
        for (Object space : spaceMap(state).values()) {
            if (!space.getClass().getSimpleName().equals("SymZ3MemorySpace")) {
                continue;
            }
            Field field = space.getClass().getDeclaredField("mmap");
            field.setAccessible(true);
            maps.add((SymZ3MemoryMap) field.get(space));
        }
        return maps;
    }

    @SuppressWarnings("unchecked")
    private List<Z3MemoryWitness> memoryWitnesses(SymZ3MemoryMap map) throws Exception {
        Field field = SymZ3MemoryMap.class.getDeclaredField("witnesses");
        field.setAccessible(true);
        return new ArrayList<>((List<Z3MemoryWitness>) field.get(map));
    }

    private List<SymZ3RecordsExecution.RecOp> branchOps(SymZ3PcodeExecutorStatePiece state) {
        return state.getOps()
            .stream()
            .filter(op -> op.op() != null && op.op().getOpcode() == PcodeOp.CBRANCH)
            .collect(Collectors.toList());
    }

    private String displayBool(Context ctx, Z3InfixPrinter z3p, String serialized) {
        try {
            BoolExpr expr = SymValueZ3.deserializeBoolExpr(ctx, serialized);
            return z3p.infix(expr.simplify());
        }
        catch (Exception e) {
            return serialized;
        }
    }

    private boolean inferredBranchTaken(String serialized) {
        return !serialized.trim().startsWith("(not");
    }

    private String pcodeId(PcodeOp op) {
        if (op.getSeqnum() == null) {
            return null;
        }
        return "ghidra:pcode:" + address(op.getSeqnum().getTarget()) + ":" +
            op.getSeqnum().getTime();
    }

    private String valueKind(String name) {
        if (name.startsWith("MEM ")) {
            return "memory";
        }
        if (name.contains(":")) {
            return "register";
        }
        return "symbolic";
    }

    private String valueSpace(String name) {
        if (name.startsWith("MEM ")) {
            return "memory";
        }
        if (name.contains(":")) {
            return "register";
        }
        return null;
    }

    private String byteLenOrNull(String name) {
        int colon = name.lastIndexOf(':');
        if (colon < 0 || colon + 1 >= name.length()) {
            return "null";
        }
        try {
            int bits = Integer.parseInt(name.substring(colon + 1).trim());
            if (bits <= 0 || bits % 8 != 0) {
                return "null";
            }
            return Integer.toString(bits / 8);
        }
        catch (NumberFormatException e) {
            return "null";
        }
    }

    private int intArg(String[] args, int index, int fallback) {
        if (args.length <= index) {
            return fallback;
        }
        try {
            return Math.max(1, Integer.parseInt(args[index]));
        }
        catch (NumberFormatException e) {
            return fallback;
        }
    }

    private void writeObjects(PrintWriter out, List<String> objects) {
        for (int i = 0; i < objects.size(); i++) {
            String suffix = i + 1 == objects.size() ? "" : ",";
            out.println(objects.get(i) + suffix);
        }
    }

    private String address(Address address) {
        if (address == null) {
            return null;
        }
        if (address.isMemoryAddress()) {
            return "0x" + Long.toHexString(address.getUnsignedOffset());
        }
        return address.getAddressSpace().getName() + ":0x" +
            Long.toHexString(address.getUnsignedOffset());
    }

    private String cleanId(String value) {
        return value.replaceAll("[^A-Za-z0-9_.:-]", "_");
    }

    private String jsonArray(List<String> values) {
        return values.stream().map(this::json).collect(Collectors.joining(", ", "[", "]"));
    }

    private String jsonOrNull(String value) {
        return value == null ? "null" : json(value);
    }

    private String json(String value) {
        if (value == null) {
            return "null";
        }
        StringBuilder builder = new StringBuilder("\"");
        for (char c : value.toCharArray()) {
            switch (c) {
                case '\\':
                    builder.append("\\\\");
                    break;
                case '"':
                    builder.append("\\\"");
                    break;
                case '\n':
                    builder.append("\\n");
                    break;
                case '\r':
                    builder.append("\\r");
                    break;
                case '\t':
                    builder.append("\\t");
                    break;
                default:
                    builder.append(c);
            }
        }
        builder.append("\"");
        return builder.toString();
    }
}
