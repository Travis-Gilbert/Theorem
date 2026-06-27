// Headless Ghidra fixture exporter for Theorem's program-analysis oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-oracle -import hello_tiny.o \
//   -postScript ExportTheoremFacts.java hello_tiny.oracle.json

import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolIterator;
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

        int symbolCount = 0;
        SymbolIterator symbolIterator = symbols.getAllSymbols(true);
        while (symbolIterator.hasNext()) {
            Symbol ignored = symbolIterator.next();
            symbolCount++;
        }

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"ghidra_version\": \"" + getGhidraVersion() + "\",");
            out.println("  \"language_id\": \"" + currentProgram.getLanguageID() + "\",");
            out.println("  \"compiler_spec_id\": \"" + currentProgram.getCompilerSpec().getCompilerSpecID() + "\",");
            out.println("  \"analysis_timeout_occurred\": false,");
            out.println("  \"function_count\": " + functionCount + ",");
            out.println("  \"import_count\": " + importCount + ",");
            out.println("  \"symbol_count\": " + symbolCount);
            out.println("}");
        }
    }
}
