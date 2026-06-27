// Headless Ghidra fixture exporter for Theorem's program-analysis oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-oracle -import hello_tiny.o \
//   -postScript ExportTheoremFacts.java hello_tiny.oracle.json

import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.SymbolTable;
import java.io.FileWriter;
import java.io.PrintWriter;

public class ExportTheoremFacts extends GhidraScript {
    @Override
    public void run() throws Exception {
        String outputPath = getScriptArgs().length > 0
            ? getScriptArgs()[0]
            : "theorem-ghidra-oracle.json";

        SymbolTable symbols = currentProgram.getSymbolTable();
        int functionCount = 0;
        for (Function ignored : currentProgram.getFunctionManager().getFunctions(true)) {
            functionCount++;
        }

        int importCount = 0;
        for (ExternalLocation ignored : symbols.getExternalLocations()) {
            importCount++;
        }

        String fixtureId = "ghidra:oracle:" + currentProgram.getName();
        String sourceUri = currentProgram.getExecutablePath();
        String evidenceId = "ghidra:program:" + currentProgram.getName();

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture_id\": " + json(fixtureId) + ",");
            out.println("  \"source_uri\": " + json(sourceUri) + ",");
            out.println("  \"export_script\": \"ExportTheoremFacts.java\",");
            out.println("  \"evidence_ids\": [" + json(evidenceId) + "],");
            out.println("  \"program_summary\": {");
            out.println("    \"ghidra_version\": " + json(getGhidraVersion()) + ",");
            out.println("    \"language_id\": " + json(currentProgram.getLanguageID().toString()) + ",");
            out.println("    \"compiler_spec_id\": " + json(currentProgram.getCompilerSpec().getCompilerSpecID().toString()) + ",");
            out.println("    \"analysis_timeout_occurred\": false,");
            out.println("    \"function_count\": " + functionCount + ",");
            out.println("    \"import_count\": " + importCount + ",");
            out.println("    \"string_count\": 0,");
            out.println("    \"cfg_edge_count\": 0");
            out.println("  }");
            out.println("}");
        }
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
