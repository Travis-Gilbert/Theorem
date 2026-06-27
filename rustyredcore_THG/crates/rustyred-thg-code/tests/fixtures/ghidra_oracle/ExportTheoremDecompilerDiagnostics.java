// Headless Ghidra decompiler diagnostic exporter for Theorem's
// program-analysis oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-decompiler-diagnostics -import hello_tiny.o \
//   -postScript ExportTheoremDecompilerDiagnostics.java hello_tiny.diagnostics.oracle.json 256 30
//
// Args:
//   0: output path
//   1: max functions to decompile, default 256
//   2: timeout seconds per function, default 30

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.SymbolTable;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

public class ExportTheoremDecompilerDiagnostics extends GhidraScript {
    private static final int DEFAULT_MAX_FUNCTIONS = 256;
    private static final int DEFAULT_TIMEOUT_SECONDS = 30;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outputPath = args.length > 0
            ? args[0]
            : "theorem-ghidra-decompiler-diagnostics-oracle.json";
        int maxFunctions = intArg(args, 1, DEFAULT_MAX_FUNCTIONS);
        int timeoutSeconds = intArg(args, 2, DEFAULT_TIMEOUT_SECONDS);

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

        List<String> diagnostics = new ArrayList<>();
        DecompInterface decompiler = new DecompInterface();
        try {
            DecompileOptions options = new DecompileOptions();
            options.setDefaultTimeout(timeoutSeconds);
            decompiler.setOptions(options);
            decompiler.toggleCCode(false);
            decompiler.toggleSyntaxTree(true);
            if (!decompiler.openProgram(currentProgram)) {
                diagnostics.add(programDiagnostic(
                    "fatal",
                    "unknown",
                    "export",
                    "Ghidra decompiler failed to open program: " + decompiler.getLastMessage(),
                    true,
                    false,
                    false));
            }
            else {
                for (Function function : functions) {
                    monitor.checkCancelled();
                    DecompileResults result =
                        decompiler.decompileFunction(function, timeoutSeconds, monitor);
                    String message = result.getErrorMessage();
                    if (message == null) {
                        message = "";
                    }
                    message = message.trim();
                    if (message.isEmpty() && result.decompileCompleted() && result.isValid()) {
                        continue;
                    }
                    diagnostics.add(functionDiagnostic(function, result, message));
                    decompiler.flushCache();
                }
            }
        }
        finally {
            decompiler.dispose();
        }

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture\": {");
            out.println("    \"fixture_id\": " + json("ghidra:decompiler_diagnostics_oracle:" +
                currentProgram.getName()) + ",");
            out.println("    \"source_uri\": " + json(currentProgram.getExecutablePath()) + ",");
            out.println("    \"export_script\": \"ExportTheoremDecompilerDiagnostics.java\",");
            out.println("    \"evidence_ids\": [" +
                json("ghidra:decompiler_diagnostics_program:" + currentProgram.getName()) + "],");
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
            out.println("  \"diagnostics\": [");
            writeObjects(out, diagnostics);
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

    private String functionDiagnostic(
            Function function,
            DecompileResults result,
            String message) {
        String entry = address(function.getEntryPoint());
        String category = inferCategory(message);
        String severity = inferSeverity(result, message);
        boolean affectsControlFlow = category.equals("flow") || category.equals("jump_table") ||
            containsAny(message, "flow", "branch", "jump");
        boolean affectsPrototype = category.equals("prototype") || category.equals("parameter_id") ||
            containsAny(message, "prototype", "parameter", "calling convention");
        boolean affectsDataFlow = category.equals("stack_pointer") || category.equals("type_recovery") ||
            category.equals("variable_merge") || containsAny(message, "stack", "type", "variable");
        List<String> evidence = new ArrayList<>();
        evidence.add("ghidra:decompiler_diagnostic:function:" + entry);
        evidence.add("ghidra:function:" + entry);

        return "    {" +
            "\"diagnostic_id\": " + json("ghidra:decompiler_diagnostic:" + entry + ":" +
                cleanId(message.isEmpty() ? severity : message)) + ", " +
            "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"address\": " + json(entry) + ", " +
            "\"category\": " + json(category) + ", " +
            "\"severity\": " + json(severity) + ", " +
            "\"placement\": \"header\", " +
            "\"message\": " + json(message.isEmpty() ? fallbackMessage(result) : message) + ", " +
            "\"source_pass\": \"DecompInterface.decompileFunction\", " +
            "\"source_rule\": null, " +
            "\"affects_control_flow\": " + affectsControlFlow + ", " +
            "\"affects_prototype\": " + affectsPrototype + ", " +
            "\"affects_data_flow\": " + affectsDataFlow + ", " +
            "\"remediation\": " +
                json("Treat this decompiler diagnostic as reconstruction uncertainty and preserve behavior with validators.") + ", " +
            "\"completed\": " + result.decompileCompleted() + ", " +
            "\"timed_out\": " + result.isTimedOut() + ", " +
            "\"cancelled\": " + result.isCancelled() + ", " +
            "\"failed_to_start\": " + result.failedToStart() + ", " +
            "\"evidence_ids\": " + jsonArray(evidence) +
            "}";
    }

    private String programDiagnostic(
            String severity,
            String category,
            String placement,
            String message,
            boolean failedToStart,
            boolean timedOut,
            boolean cancelled) {
        return "    {" +
            "\"diagnostic_id\": " + json("ghidra:decompiler_diagnostic:program:" +
                cleanId(message)) + ", " +
            "\"function_id\": null, " +
            "\"entry_point\": \"\", " +
            "\"address\": null, " +
            "\"category\": " + json(category) + ", " +
            "\"severity\": " + json(severity) + ", " +
            "\"placement\": " + json(placement) + ", " +
            "\"message\": " + json(message) + ", " +
            "\"source_pass\": \"DecompInterface.openProgram\", " +
            "\"source_rule\": null, " +
            "\"affects_control_flow\": false, " +
            "\"affects_prototype\": false, " +
            "\"affects_data_flow\": false, " +
            "\"remediation\": " +
                json("Ghidra decompiler did not initialize; run diagnostics before trusting recovered behavior.") + ", " +
            "\"completed\": false, " +
            "\"timed_out\": " + timedOut + ", " +
            "\"cancelled\": " + cancelled + ", " +
            "\"failed_to_start\": " + failedToStart + ", " +
            "\"evidence_ids\": [" + json("ghidra:decompiler_diagnostic:program") + "]" +
            "}";
    }

    private String inferSeverity(DecompileResults result, String message) {
        String lower = message.toLowerCase(Locale.ROOT);
        if (result.failedToStart() || lower.contains("fatal") || lower.contains("crash")) {
            return "fatal";
        }
        if (result.isTimedOut() || !result.decompileCompleted() || lower.contains("error")) {
            return "error";
        }
        return "warning";
    }

    private String inferCategory(String message) {
        String lower = message.toLowerCase(Locale.ROOT);
        if (containsAny(lower, "jump", "switch")) {
            return "jump_table";
        }
        if (containsAny(lower, "stack", "spacebase")) {
            return "stack_pointer";
        }
        if (containsAny(lower, "prototype", "calling convention", "extrapop")) {
            return "prototype";
        }
        if (containsAny(lower, "paramid", "parameter")) {
            return "parameter_id";
        }
        if (containsAny(lower, "type", "datatype")) {
            return "type_recovery";
        }
        if (containsAny(lower, "variable", "merge")) {
            return "variable_merge";
        }
        if (containsAny(lower, "export", "c markup")) {
            return "export";
        }
        if (containsAny(lower, "flow", "branch", "instruction", "unimplemented")) {
            return "flow";
        }
        return "unknown";
    }

    private String fallbackMessage(DecompileResults result) {
        if (result.isTimedOut()) {
            return "Ghidra decompiler timed out";
        }
        if (result.isCancelled()) {
            return "Ghidra decompiler was cancelled";
        }
        if (result.failedToStart()) {
            return "Ghidra decompiler failed to start";
        }
        if (!result.decompileCompleted()) {
            return "Ghidra decompiler did not complete";
        }
        return "Ghidra decompiler emitted an unspecified diagnostic";
    }

    private boolean containsAny(String value, String... needles) {
        String lower = value.toLowerCase(Locale.ROOT);
        for (String needle : needles) {
            if (lower.contains(needle)) {
                return true;
            }
        }
        return false;
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
        String cleaned = value.replaceAll("[^A-Za-z0-9_.:-]", "_");
        return cleaned.length() > 80 ? cleaned.substring(0, 80) : cleaned;
    }

    private String jsonArray(List<String> values) {
        List<String> encoded = new ArrayList<>();
        for (String value : values) {
            encoded.add(json(value));
        }
        return "[" + String.join(", ", encoded) + "]";
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
