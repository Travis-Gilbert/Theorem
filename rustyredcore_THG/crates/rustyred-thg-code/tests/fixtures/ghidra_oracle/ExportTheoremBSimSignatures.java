// Headless Ghidra BSim/decompiler signature exporter for Theorem's
// program-analysis oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-bsim-signatures -import hello_tiny.o \
//   -postScript ExportTheoremBSimSignatures.java hello_tiny.bsim.oracle.json 256 30 7
//
// Args:
//   0: output path
//   1: max functions to signature, default 256
//   2: timeout seconds per function, default 30
//   3: signature settings bitmask, default 7

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.signature.DebugSignature;
import ghidra.app.decompiler.signature.SignatureResult;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.SymbolTable;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.List;

public class ExportTheoremBSimSignatures extends GhidraScript {
    private static final int DEFAULT_MAX_FUNCTIONS = 256;
    private static final int DEFAULT_TIMEOUT_SECONDS = 30;
    private static final int DEFAULT_SIGNATURE_SETTINGS = 7;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outputPath = args.length > 0
            ? args[0]
            : "theorem-ghidra-bsim-signatures-oracle.json";
        int maxFunctions = intArg(args, 1, DEFAULT_MAX_FUNCTIONS);
        int timeoutSeconds = Math.max(1, intArg(args, 2, DEFAULT_TIMEOUT_SECONDS));
        int signatureSettings = intArg(args, 3, DEFAULT_SIGNATURE_SETTINGS);

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

        List<String> semanticSignatures = new ArrayList<>();
        DecompInterface decompiler = new DecompInterface();
        try {
            DecompileOptions options = new DecompileOptions();
            options.setDefaultTimeout(timeoutSeconds);
            decompiler.setOptions(options);
            decompiler.toggleCCode(false);
            decompiler.toggleSyntaxTree(false);
            decompiler.setSignatureSettings(signatureSettings);
            if (!decompiler.openProgram(currentProgram)) {
                throw new IllegalStateException(
                    "Ghidra decompiler failed to open program: " + decompiler.getLastMessage());
            }

            short majorVersion = decompiler.getMajorVersion();
            short minorVersion = decompiler.getMinorVersion();
            int activeSettings = decompiler.getSignatureSettings();

            for (Function function : functions) {
                monitor.checkCancelled();
                SignatureResult result =
                    decompiler.generateSignatures(function, true, timeoutSeconds, monitor);
                if (result == null) {
                    println("Skipping BSim signature for " + function.getName() + ": " +
                        decompiler.getLastMessage());
                    decompiler.flushCache();
                    continue;
                }
                ArrayList<DebugSignature> debug =
                    decompiler.debugSignatures(function, timeoutSeconds, monitor);
                semanticSignatures.add(functionSignature(
                    function,
                    result,
                    debug,
                    activeSettings,
                    majorVersion,
                    minorVersion));
                decompiler.flushCache();
            }
        }
        finally {
            decompiler.dispose();
        }

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture\": {");
            out.println("    \"fixture_id\": " + json("ghidra:bsim_signature_oracle:" +
                currentProgram.getName()) + ",");
            out.println("    \"source_uri\": " + json(currentProgram.getExecutablePath()) + ",");
            out.println("    \"export_script\": \"ExportTheoremBSimSignatures.java\",");
            out.println("    \"evidence_ids\": [" +
                json("ghidra:bsim_signature_program:" + currentProgram.getName()) + "],");
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
            out.println("  \"semantic_signatures\": [");
            writeObjects(out, semanticSignatures);
            out.println("  ],");
            out.println("  \"function_id_signatures\": [");
            writeObjects(out, new ArrayList<>());
            out.println("  ]");
            out.println("}");
        }
    }

    private String functionSignature(
            Function function,
            SignatureResult result,
            ArrayList<DebugSignature> debug,
            int signatureSettings,
            short majorVersion,
            short minorVersion) {
        String entry = address(function.getEntryPoint());
        return "    {" +
            "\"signature_id\": " + json("ghidra:semantic_signature:" + entry + ":bsim") + ", " +
            "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"function_name\": " + json(function.getName()) + ", " +
            "\"feature_version\": \"ghidra-bsim-signature-v0\", " +
            "\"signature_settings\": " + signatureSettings + ", " +
            "\"decompiler_major_version\": " + majorVersion + ", " +
            "\"decompiler_minor_version\": " + minorVersion + ", " +
            "\"feature_hashes\": " + featureHashes(result.features) + ", " +
            "\"debug_features\": " + debugFeatures(debug) + ", " +
            "\"call_targets\": " + callTargets(result.calllist) + ", " +
            "\"has_unimplemented\": " + result.hasunimplemented + ", " +
            "\"has_bad_data\": " + result.hasbaddata + ", " +
            "\"evidence_ids\": [" + json("ghidra:bsim_signature:function:" + entry) + "]" +
            "}";
    }

    private String featureHashes(int[] hashes) {
        if (hashes == null || hashes.length == 0) {
            return "[]";
        }
        List<String> encoded = new ArrayList<>();
        for (int hash : hashes) {
            encoded.add(json(unsignedHex(hash)));
        }
        return "[" + String.join(", ", encoded) + "]";
    }

    private String debugFeatures(ArrayList<DebugSignature> debug) {
        if (debug == null || debug.isEmpty()) {
            return "[]";
        }
        List<String> encoded = new ArrayList<>();
        for (DebugSignature signature : debug) {
            StringBuffer raw = new StringBuffer();
            signature.printRaw(currentProgram.getLanguage(), raw);
            encoded.add("{" +
                "\"hash\": " + json(unsignedHex(signature.hash)) + ", " +
                "\"kind\": " + json(signature.getClass().getSimpleName()) + ", " +
                "\"raw\": " + json(raw.toString()) +
                "}");
        }
        return "[" + String.join(", ", encoded) + "]";
    }

    private String callTargets(ArrayList<Address> addresses) {
        if (addresses == null || addresses.isEmpty()) {
            return "[]";
        }
        List<String> encoded = new ArrayList<>();
        for (Address address : addresses) {
            encoded.add(json(address(address)));
        }
        return "[" + String.join(", ", encoded) + "]";
    }

    private String unsignedHex(int value) {
        return String.format("0x%08x", Integer.toUnsignedLong(value));
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
        return "\"" + value
            .replace("\\", "\\\\")
            .replace("\"", "\\\"")
            .replace("\n", "\\n")
            .replace("\r", "\\r")
            .replace("\t", "\\t") + "\"";
    }
}
