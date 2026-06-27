// Headless Ghidra FunctionID hash exporter for Theorem's program-analysis
// oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-fid -import hello_tiny.o \
//   -postScript ExportTheoremFunctionId.java hello_tiny.fid.oracle.json 256
//
// Args:
//   0: output path
//   1: max functions to hash, default 256

import ghidra.app.script.GhidraScript;
import ghidra.feature.fid.hash.FidHashQuad;
import ghidra.feature.fid.service.FidService;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.mem.MemoryAccessException;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.SymbolTable;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.List;

public class ExportTheoremFunctionId extends GhidraScript {
    private static final int DEFAULT_MAX_FUNCTIONS = 256;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outputPath = args.length > 0
            ? args[0]
            : "theorem-ghidra-function-id-oracle.json";
        int maxFunctions = intArg(args, 1, DEFAULT_MAX_FUNCTIONS);

        List<Function> functions = new ArrayList<>();
        if (maxFunctions > 0) {
            for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
                if (functions.size() >= maxFunctions) {
                    break;
                }
                functions.add(function);
            }
        }

        SymbolTable symbols = currentProgram.getSymbolTable();
        int importCount = 0;
        for (ExternalLocation ignored : symbols.getExternalLocations()) {
            importCount++;
        }

        FidService fidService = new FidService();
        List<String> functionIdFacts = new ArrayList<>();
        for (Function function : functions) {
            monitor.checkCancelled();
            try {
                FidHashQuad hash = fidService.hashFunction(function);
                if (hash == null) {
                    continue;
                }
                functionIdFacts.add(functionIdFact(function, hash, fidService));
            }
            catch (MemoryAccessException e) {
                println("Skipping FunctionID hash for " + function.getName() + ": " +
                    e.getMessage());
            }
        }

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture\": {");
            out.println("    \"fixture_id\": " + json("ghidra:function_id_oracle:" +
                currentProgram.getName()) + ",");
            out.println("    \"source_uri\": " + json(currentProgram.getExecutablePath()) + ",");
            out.println("    \"export_script\": \"ExportTheoremFunctionId.java\",");
            out.println("    \"evidence_ids\": [" +
                json("ghidra:function_id_program:" + currentProgram.getName()) + "],");
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
            out.println("  \"symbolic_summaries\": [],");
            out.println("  \"diagnostics\": [],");
            out.println("  \"semantic_signatures\": [],");
            out.println("  \"function_id_signatures\": [");
            writeObjects(out, functionIdFacts);
            out.println("  ]");
            out.println("}");
        }
    }

    private String functionIdFact(Function function, FidHashQuad hash, FidService fidService) {
        String entry = address(function.getEntryPoint());
        return "    {" +
            "\"fid_id\": " + json("ghidra:function_id:" + entry) + ", " +
            "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"function_name\": " + json(function.getName()) + ", " +
            "\"feature_version\": \"ghidra-function-id-v0\", " +
            "\"hash_algorithm\": \"fnv1a64\", " +
            "\"short_hash_code_unit_length\": " + fidService.getShortHashCodeUnitLength() + ", " +
            "\"medium_hash_code_unit_length\": " +
                fidService.getMediumHashCodeUnitLengthLimit() + ", " +
            "\"code_unit_size\": " + hash.getCodeUnitSize() + ", " +
            "\"full_hash\": " + json(unsignedHex(hash.getFullHash())) + ", " +
            "\"specific_hash_additional_size\": " +
                Byte.toUnsignedInt(hash.getSpecificHashAdditionalSize()) + ", " +
            "\"specific_hash\": " + json(unsignedHex(hash.getSpecificHash())) + ", " +
            "\"matches\": [], " +
            "\"evidence_ids\": [" + json("ghidra:function_id:function:" + entry) + "]" +
            "}";
    }

    private String unsignedHex(long value) {
        return String.format("0x%016x", value);
    }

    private void writeObjects(PrintWriter out, List<String> objects) {
        for (int i = 0; i < objects.size(); i++) {
            String suffix = i + 1 == objects.size() ? "" : ",";
            out.println(objects.get(i) + suffix);
        }
    }

    private int intArg(String[] args, int index, int fallback) {
        if (args.length <= index) {
            return fallback;
        }
        try {
            return Integer.parseInt(args[index]);
        }
        catch (NumberFormatException ignored) {
            return fallback;
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

    private String json(String value) {
        if (value == null) {
            return "null";
        }
        return "\"" + value
            .replace("\\", "\\\\")
            .replace("\"", "\\\"")
            .replace("\n", "\\n")
            .replace("\r", "\\r")
            .replace("\t", "\\t") + "\"";
    }
}
