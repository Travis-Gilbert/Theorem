// Headless Ghidra fixture exporter for Theorem's program-analysis oracle loop.
//
// Invocation shape:
// analyzeHeadless <project-dir> theorem-oracle -import hello_tiny.o \
//   -postScript ExportTheoremFacts.java hello_tiny.oracle.json 256 30
//
// Args:
//   0: output path
//   1: max functions to decompile for jump-table recovery, default 256
//   2: timeout seconds per function, default 30

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.decompiler.ClangBitFieldToken;
import ghidra.app.decompiler.ClangFieldToken;
import ghidra.app.decompiler.ClangNode;
import ghidra.app.decompiler.ClangToken;
import ghidra.app.decompiler.ClangTokenGroup;
import ghidra.app.cmd.function.CallDepthChangeInfo;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressIterator;
import ghidra.program.model.address.AddressSetView;
import ghidra.program.model.address.AddressSpace;
import ghidra.program.model.data.Array;
import ghidra.program.model.data.BitFieldDataType;
import ghidra.program.model.data.Composite;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeComponent;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.Pointer;
import ghidra.program.model.data.Structure;
import ghidra.program.model.data.TypeDef;
import ghidra.program.model.data.Union;
import ghidra.program.model.lang.PrototypeModel;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Bookmark;
import ghidra.program.model.listing.BookmarkManager;
import ghidra.program.model.listing.CommentType;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.listing.Parameter;
import ghidra.program.model.listing.StackFrame;
import ghidra.program.model.listing.Variable;
import ghidra.program.model.listing.VariableStorage;
import ghidra.program.model.pcode.GlobalSymbolMap;
import ghidra.program.model.pcode.HighParamID;
import ghidra.program.model.pcode.HighFunction;
import ghidra.program.model.pcode.HighSymbol;
import ghidra.program.model.pcode.HighVariable;
import ghidra.program.model.pcode.JumpTable;
import ghidra.program.model.pcode.LocalSymbolMap;
import ghidra.program.model.pcode.ParamMeasure;
import ghidra.program.model.pcode.PcodeOp;
import ghidra.program.model.pcode.Varnode;
import ghidra.program.model.symbol.Equate;
import ghidra.program.model.symbol.EquateReference;
import ghidra.program.model.symbol.EquateSymbol;
import ghidra.program.model.symbol.EquateTable;
import ghidra.program.model.symbol.ExternalManager;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;
import ghidra.program.model.symbol.SymbolTable;
import java.io.FileWriter;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Set;

public class ExportTheoremFacts extends GhidraScript {
    private static final int DEFAULT_CASE_VALUE = 0xbad1abe1;
    private static final int DEFAULT_MAX_DECOMPILE_FUNCTIONS = 256;
    private static final int DEFAULT_TIMEOUT_SECONDS = 30;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outputPath = args.length > 0
            ? args[0]
            : "theorem-ghidra-oracle.json";
        int maxDecompileFunctions =
            intArg(args, 1, DEFAULT_MAX_DECOMPILE_FUNCTIONS);
        int timeoutSeconds = intArg(args, 2, DEFAULT_TIMEOUT_SECONDS);

        Listing listing = currentProgram.getListing();
        ReferenceManager references = currentProgram.getReferenceManager();
        SymbolTable symbols = currentProgram.getSymbolTable();
        List<Function> functions = new ArrayList<>();
        for (Function function : currentProgram.getFunctionManager().getFunctions(true)) {
            functions.add(function);
        }

        int importCount = 0;
        for (ExternalLocation ignored : symbols.getExternalLocations()) {
            importCount++;
        }

        String fixtureId = "ghidra:oracle:" + currentProgram.getName();
        String sourceUri = currentProgram.getExecutablePath();
        String evidenceId = "ghidra:program:" + currentProgram.getName();
        List<String> functionFacts = new ArrayList<>();
        List<String> pcodeFacts = new ArrayList<>();
        List<String> referenceFacts = new ArrayList<>();
        List<String> callEdgeFacts = new ArrayList<>();
        List<String> jumpTableFacts = new ArrayList<>();
        List<String> equateFacts = new ArrayList<>();
        List<String> externalLinkageFacts = new ArrayList<>();
        List<String> functionPrototypeFacts = new ArrayList<>();
        List<String> dataTypeFacts = new ArrayList<>();
        List<String> highVariableFacts = new ArrayList<>();
        List<String> stackFrameFacts = new ArrayList<>();
        List<String> parameterMeasureFacts = new ArrayList<>();
        List<String> callStackEffectFacts = new ArrayList<>();
        List<String> annotationFacts = new ArrayList<>();
        List<String> structureFieldAccessFacts = new ArrayList<>();
        List<String> symbolicSummaries = new ArrayList<>();
        Set<String> seenReferences = new HashSet<>();
        Set<String> seenCallEdges = new HashSet<>();

        for (Function function : functions) {
            AddressSetView body = function.getBody();
            String entry = address(function.getEntryPoint());
            String bodyStart = address(body.getMinAddress());
            String bodyEnd = address(body.getMaxAddress());
            functionFacts.add("    {" +
                "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
                "\"entry_point\": " + json(entry) + ", " +
                "\"name\": " + json(function.getName()) + ", " +
                "\"body_start\": " + json(bodyStart) + ", " +
                "\"body_end\": " + json(bodyEnd) + ", " +
                "\"evidence_ids\": [" + json("ghidra:function:" + entry) + "]" +
                "}");

            for (Instruction instruction : listing.getInstructions(body, true)) {
                Address instructionAddress = instruction.getMinAddress();
                for (PcodeOp op : instruction.getPcode()) {
                    String pcodeAddress = address(op.getSeqnum() == null
                        ? instructionAddress
                        : op.getSeqnum().getTarget());
                    long sequence = op.getSeqnum() == null ? 0 : op.getSeqnum().getTime();
                    pcodeFacts.add("    {" +
                        "\"pcode_id\": " + json("ghidra:pcode:" + pcodeAddress + ":" + sequence) + ", " +
                        "\"address\": " + json(pcodeAddress) + ", " +
                        "\"sequence\": " + sequence + ", " +
                        "\"opcode\": " + json(op.getMnemonic()) + ", " +
                        "\"ghidra_opcode_id\": " + op.getOpcode() + ", " +
                        "\"inputs\": " + varnodeArray(op.getInputs()) + ", " +
                        "\"output\": " + jsonOrNull(varnode(op.getOutput())) + ", " +
                        "\"evidence_ids\": [" + json("ghidra:pcode:" + pcodeAddress) + "]" +
                        "}");
                }

                for (Reference reference : references.getReferencesFrom(instructionAddress)) {
                    String from = address(reference.getFromAddress());
                    String to = address(reference.getToAddress());
                    String referenceKey = from + "->" + to + ":" +
                        reference.getReferenceType().toString() + ":" + reference.getOperandIndex();
                    if (seenReferences.add(referenceKey)) {
                        referenceFacts.add("    {" +
                            "\"reference_id\": " + json("ghidra:ref:" + referenceKey) + ", " +
                            "\"from_address\": " + json(from) + ", " +
                            "\"to_address\": " + json(to) + ", " +
                            "\"reference_type\": " + json(reference.getReferenceType().toString()) + ", " +
                            "\"operand_index\": " + reference.getOperandIndex() + ", " +
                            "\"is_primary\": " + reference.isPrimary() + ", " +
                            "\"source_type\": " + json(reference.getSource().toString()) + ", " +
                            "\"semantic_roles\": " + semanticRoles(reference) + ", " +
                            "\"is_external\": " + reference.isExternalReference() + ", " +
                            "\"is_memory\": " + reference.isMemoryReference() + ", " +
                            "\"is_register\": " + reference.isRegisterReference() + ", " +
                            "\"is_stack\": " + reference.isStackReference() + ", " +
                            "\"evidence_ids\": [" + json("ghidra:reference:" + from) + "]" +
                            "}");
                    }

                    if (reference.getReferenceType().isCall()) {
                        Function targetFunction =
                            currentProgram.getFunctionManager().getFunctionContaining(reference.getToAddress());
                        if (targetFunction == null) {
                            continue;
                        }
                        String targetEntry = address(targetFunction.getEntryPoint());
                        String callsite = address(reference.getFromAddress());
                        String callEdgeKey = entry + "->" + targetEntry + "@" + callsite;
                        if (seenCallEdges.add(callEdgeKey)) {
                            callEdgeFacts.add("    {" +
                                "\"edge_id\": " + json("ghidra:call:" + callEdgeKey) + ", " +
                                "\"source_entry\": " + json(entry) + ", " +
                                "\"target_entry\": " + json(targetEntry) + ", " +
                                "\"callsite_address\": " + json(callsite) + ", " +
                                "\"evidence_ids\": [" + json("ghidra:call:" + callsite) + "]" +
                                "}");
                        }
                    }
                }
            }
        }

        collectJumpTables(functions, maxDecompileFunctions, timeoutSeconds, jumpTableFacts);
        collectEquates(equateFacts);
        collectExternalLinkages(functions, externalLinkageFacts);
        collectFunctionPrototypes(functions, functionPrototypeFacts);
        collectDataTypes(dataTypeFacts);
        collectHighVariables(functions, maxDecompileFunctions, timeoutSeconds, highVariableFacts);
        collectStackFrames(functions, stackFrameFacts);
        collectParameterMeasures(functions, maxDecompileFunctions, timeoutSeconds, parameterMeasureFacts);
        collectCallStackEffects(functions, listing, callStackEffectFacts);
        collectAnnotations(listing, annotationFacts);
        collectStructureFieldAccesses(
            functions,
            maxDecompileFunctions,
            timeoutSeconds,
            structureFieldAccessFacts);

        try (PrintWriter out = new PrintWriter(new FileWriter(outputPath))) {
            out.println("{");
            out.println("  \"fixture\": {");
            out.println("    \"fixture_id\": " + json(fixtureId) + ",");
            out.println("    \"source_uri\": " + json(sourceUri) + ",");
            out.println("    \"export_script\": \"ExportTheoremFacts.java\",");
            out.println("    \"evidence_ids\": [" + json(evidenceId) + "],");
            out.println("    \"program_summary\": {");
            out.println("      \"ghidra_version\": " + json(getGhidraVersion()) + ",");
            out.println("      \"language_id\": " + json(currentProgram.getLanguageID().toString()) + ",");
            out.println("      \"compiler_spec_id\": " +
                json(currentProgram.getCompilerSpec().getCompilerSpecID().toString()) + ",");
            out.println("      \"analysis_timeout_occurred\": false,");
            out.println("      \"function_count\": " + functions.size() + ",");
            out.println("      \"import_count\": " + importCount + ",");
            out.println("      \"string_count\": 0,");
            out.println("      \"cfg_edge_count\": " + callEdgeFacts.size());
            out.println("    }");
            out.println("  },");
            out.println("  \"functions\": [");
            writeObjects(out, functionFacts);
            out.println("  ],");
            out.println("  \"pcode_ops\": [");
            writeObjects(out, pcodeFacts);
            out.println("  ],");
            out.println("  \"references\": [");
            writeObjects(out, referenceFacts);
            out.println("  ],");
            out.println("  \"call_edges\": [");
            writeObjects(out, callEdgeFacts);
            out.println("  ],");
            out.println("  \"jump_tables\": [");
            writeObjects(out, jumpTableFacts);
            out.println("  ],");
            out.println("  \"equates\": [");
            writeObjects(out, equateFacts);
            out.println("  ],");
            out.println("  \"external_linkages\": [");
            writeObjects(out, externalLinkageFacts);
            out.println("  ],");
            out.println("  \"function_prototypes\": [");
            writeObjects(out, functionPrototypeFacts);
            out.println("  ],");
            out.println("  \"data_types\": [");
            writeObjects(out, dataTypeFacts);
            out.println("  ],");
            out.println("  \"high_variables\": [");
            writeObjects(out, highVariableFacts);
            out.println("  ],");
            out.println("  \"stack_frames\": [");
            writeObjects(out, stackFrameFacts);
            out.println("  ],");
            out.println("  \"parameter_measures\": [");
            writeObjects(out, parameterMeasureFacts);
            out.println("  ],");
            out.println("  \"call_stack_effects\": [");
            writeObjects(out, callStackEffectFacts);
            out.println("  ],");
            out.println("  \"annotations\": [");
            writeObjects(out, annotationFacts);
            out.println("  ],");
            out.println("  \"structure_field_accesses\": [");
            writeObjects(out, structureFieldAccessFacts);
            out.println("  ],");
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

    private void writeObjects(PrintWriter out, List<String> objects) {
        for (int i = 0; i < objects.size(); i++) {
            String suffix = i + 1 == objects.size() ? "" : ",";
            out.println(objects.get(i) + suffix);
        }
    }

    private void collectAnnotations(Listing listing, List<String> annotationFacts)
            throws Exception {
        collectCommentAnnotations(listing, annotationFacts);
        collectBookmarkAnnotations(annotationFacts);
    }

    private void collectCommentAnnotations(Listing listing, List<String> annotationFacts)
            throws Exception {
        for (CommentType type : CommentType.values()) {
            AddressIterator addresses =
                listing.getCommentAddressIterator(type, currentProgram.getMemory(), true);
            while (addresses.hasNext()) {
                monitor.checkCancelled();
                Address commentAddress = addresses.next();
                String comment = emptyToNull(listing.getComment(type, commentAddress));
                if (comment == null) {
                    continue;
                }
                Function function =
                    currentProgram.getFunctionManager().getFunctionContaining(commentAddress);
                String address = address(commentAddress);
                String entry = functionEntry(function);
                annotationFacts.add("    {" +
                    "\"annotation_id\": " +
                        json("ghidra:annotation:comment:" + address + ":" +
                            type.name().toLowerCase()) + ", " +
                    "\"kind\": \"comment\", " +
                    "\"function_id\": " + jsonOrNull(functionId(function)) + ", " +
                    "\"entry_point\": " + jsonOrNull(entry) + ", " +
                    "\"address\": " + json(address) + ", " +
                    "\"comment_type\": " + json(type.name()) + ", " +
                    "\"bookmark_id\": null, " +
                    "\"bookmark_type\": null, " +
                    "\"bookmark_category\": null, " +
                    "\"message\": " + json(comment) + ", " +
                    "\"source_api\": \"Listing.getComment\", " +
                    "\"evidence_ids\": [" +
                        json("ghidra:annotation:comment:" + address) + "]" +
                    "}");
            }
        }
    }

    private void collectBookmarkAnnotations(List<String> annotationFacts) throws Exception {
        BookmarkManager bookmarks = currentProgram.getBookmarkManager();
        Iterator<Bookmark> iterator = bookmarks.getBookmarksIterator();
        while (iterator.hasNext()) {
            monitor.checkCancelled();
            Bookmark bookmark = iterator.next();
            Address bookmarkAddress = bookmark.getAddress();
            Function function =
                currentProgram.getFunctionManager().getFunctionContaining(bookmarkAddress);
            String address = address(bookmarkAddress);
            String entry = functionEntry(function);
            annotationFacts.add("    {" +
                "\"annotation_id\": " +
                    json("ghidra:annotation:bookmark:" + bookmark.getId()) + ", " +
                "\"kind\": \"bookmark\", " +
                "\"function_id\": " + jsonOrNull(functionId(function)) + ", " +
                "\"entry_point\": " + jsonOrNull(entry) + ", " +
                "\"address\": " + json(address) + ", " +
                "\"comment_type\": null, " +
                "\"bookmark_id\": " + bookmark.getId() + ", " +
                "\"bookmark_type\": " + jsonOrNull(bookmark.getTypeString()) + ", " +
                "\"bookmark_category\": " + jsonOrNull(bookmark.getCategory()) + ", " +
                "\"message\": " + json(emptyToNull(bookmark.getComment()) == null
                    ? "Ghidra bookmark"
                    : emptyToNull(bookmark.getComment())) + ", " +
                "\"source_api\": \"BookmarkManager.getBookmarksIterator\", " +
                "\"evidence_ids\": [" +
                    json("ghidra:annotation:bookmark:" + bookmark.getId()) + "]" +
                "}");
        }
    }

    private void collectDataTypes(List<String> dataTypeFacts) throws Exception {
        DataTypeManager dataTypes = currentProgram.getDataTypeManager();
        Iterator<DataType> iterator = dataTypes.getAllDataTypes();
        Set<String> seenTypes = new HashSet<>();
        while (iterator.hasNext()) {
            monitor.checkCancelled();
            DataType dataType = iterator.next();
            if (dataType == null) {
                continue;
            }
            String typeId = dataTypeId(dataType);
            if (!seenTypes.add(typeId)) {
                continue;
            }
            dataTypeFacts.add(dataTypeFact(dataType));
        }
    }

    private String dataTypeFact(DataType dataType) {
        String typeId = dataTypeId(dataType);
        DataType baseType = baseDataType(dataType);
        DataType elementType = elementDataType(dataType);
        Composite composite = dataType instanceof Composite
            ? (Composite) dataType
            : null;
        ghidra.program.model.data.Enum enumType =
            dataType instanceof ghidra.program.model.data.Enum
                ? (ghidra.program.model.data.Enum) dataType
                : null;

        return "    {" +
            "\"type_id\": " + json(typeId) + ", " +
            "\"name\": " + json(dataType.getName()) + ", " +
            "\"display_name\": " + jsonOrNull(dataType.getDisplayName()) + ", " +
            "\"kind\": " + json(dataTypeKind(dataType)) + ", " +
            "\"category_path\": " + jsonOrNull(categoryPath(dataType)) + ", " +
            "\"path_name\": " + jsonOrNull(dataType.getPathName()) + ", " +
            "\"universal_id\": " + jsonOrNull(universalId(dataType)) + ", " +
            "\"byte_len\": " + nonNegativeIntegerOrNull(dataType.getLength()) + ", " +
            "\"aligned_byte_len\": " + nonNegativeIntegerOrNull(dataType.getAlignedLength()) + ", " +
            "\"component_count\": " + (composite == null ? 0 : composite.getNumDefinedComponents()) + ", " +
            "\"packing_enabled\": " + booleanOrNull(composite == null ? null : composite.isPackingEnabled()) + ", " +
            "\"packing_value\": " + integerOrNull(
                composite != null && composite.hasExplicitPackingValue()
                    ? composite.getExplicitPackingValue()
                    : null) + ", " +
            "\"minimum_alignment\": " + integerOrNull(
                composite != null && composite.hasExplicitMinimumAlignment()
                    ? composite.getExplicitMinimumAlignment()
                    : null) + ", " +
            "\"not_yet_defined\": " + dataType.isNotYetDefined() + ", " +
            "\"zero_length\": " + dataType.isZeroLength() + ", " +
            "\"base_type_id\": " + jsonOrNull(dataTypeId(baseType)) + ", " +
            "\"base_type_name\": " + jsonOrNull(displayName(baseType)) + ", " +
            "\"element_type_id\": " + jsonOrNull(dataTypeId(elementType)) + ", " +
            "\"element_type_name\": " + jsonOrNull(displayName(elementType)) + ", " +
            "\"element_count\": " + longOrNull(elementCount(dataType)) + ", " +
            "\"element_byte_len\": " + longOrNull(elementByteLen(dataType)) + ", " +
            "\"enum_signed\": " + booleanOrNull(enumType == null ? null : enumType.isSigned()) + ", " +
            "\"enum_values\": " + nestedObjectArray(enumValueFacts(typeId, enumType), "    ") + ", " +
            "\"fields\": " + nestedObjectArray(dataTypeFieldFacts(typeId, composite), "    ") + ", " +
            "\"hard_dependency_ids\": " + stringArray(hardDataTypeDependencies(dataType)) + ", " +
            "\"soft_dependency_ids\": " + stringArray(softDataTypeDependencies(dataType)) + ", " +
            "\"evidence_ids\": [" + json("ghidra:datatype:" + nullToToken(dataType.getPathName())) + "]" +
            "}";
    }

    private List<String> dataTypeFieldFacts(String ownerTypeId, Composite composite) {
        List<String> fields = new ArrayList<>();
        if (composite == null) {
            return fields;
        }
        for (DataTypeComponent component : composite.getDefinedComponents()) {
            DataType fieldType = component.getDataType();
            BitFieldDataType bitField = fieldType instanceof BitFieldDataType
                ? (BitFieldDataType) fieldType
                : null;
            String fieldName = component.getFieldName();
            if (fieldName == null) {
                fieldName = component.getDefaultFieldName();
            }
            String fieldId = ownerTypeId + ":field:" + component.getOrdinal() +
                ":0x" + Integer.toHexString(component.getOffset());
            fields.add("      {" +
                "\"field_id\": " + json("ghidra:datatype_field:" + nullToToken(fieldId)) + ", " +
                "\"name\": " + jsonOrNull(fieldName) + ", " +
                "\"ordinal\": " + component.getOrdinal() + ", " +
                "\"offset\": " + component.getOffset() + ", " +
                "\"byte_len\": " + nonNegativeIntegerOrNull(component.getLength()) + ", " +
                "\"bit_offset\": " + integerOrNull(
                    bitField == null ? null : bitField.getBitOffset()) + ", " +
                "\"bit_size\": " + integerOrNull(
                    bitField == null ? null : bitField.getBitSize()) + ", " +
                "\"data_type_id\": " + jsonOrNull(dataTypeId(
                    bitField == null ? fieldType : bitField.getBaseDataType())) + ", " +
                "\"data_type_name\": " + jsonOrNull(displayName(fieldType)) + ", " +
                "\"comment\": " + jsonOrNull(component.getComment()) + ", " +
                "\"evidence_ids\": [" + json("ghidra:datatype_field:" + nullToToken(fieldId)) + "]" +
                "}");
        }
        return fields;
    }

    private List<String> enumValueFacts(
            String ownerTypeId,
            ghidra.program.model.data.Enum enumType) {
        List<String> values = new ArrayList<>();
        if (enumType == null) {
            return values;
        }
        for (String name : enumType.getNames()) {
            long value = enumType.getValue(name);
            String valueId = ownerTypeId + ":enum:" + nullToToken(name);
            values.add("      {" +
                "\"value_id\": " + json("ghidra:enum_value:" + nullToToken(valueId)) + ", " +
                "\"name\": " + json(name) + ", " +
                "\"value\": " + value + ", " +
                "\"comment\": " + jsonOrNull(emptyToNull(enumType.getComment(name))) + ", " +
                "\"evidence_ids\": [" + json("ghidra:enum_value:" + nullToToken(valueId)) + "]" +
                "}");
        }
        return values;
    }

    private List<String> hardDataTypeDependencies(DataType dataType) {
        List<String> dependencies = new ArrayList<>();
        DataType elementType = elementDataType(dataType);
        if (elementType != null) {
            dependencies.add(dataTypeId(elementType));
        }
        if (dataType instanceof Composite) {
            for (DataTypeComponent component : ((Composite) dataType).getDefinedComponents()) {
                DataType componentType = component.getDataType();
                if (componentType instanceof BitFieldDataType) {
                    componentType = ((BitFieldDataType) componentType).getBaseDataType();
                }
                dependencies.add(dataTypeId(componentType));
            }
        }
        return uniqueStringsWithout(dependencies, dataTypeId(dataType));
    }

    private List<String> softDataTypeDependencies(DataType dataType) {
        List<String> dependencies = new ArrayList<>();
        DataType baseType = baseDataType(dataType);
        if (baseType != null) {
            dependencies.add(dataTypeId(baseType));
        }
        return uniqueStringsWithout(dependencies, dataTypeId(dataType));
    }

    private DataType baseDataType(DataType dataType) {
        if (dataType instanceof TypeDef) {
            return ((TypeDef) dataType).getDataType();
        }
        if (dataType instanceof Pointer) {
            return ((Pointer) dataType).getDataType();
        }
        if (dataType instanceof BitFieldDataType) {
            return ((BitFieldDataType) dataType).getBaseDataType();
        }
        return null;
    }

    private DataType elementDataType(DataType dataType) {
        if (dataType instanceof Array) {
            return ((Array) dataType).getDataType();
        }
        return null;
    }

    private Long elementCount(DataType dataType) {
        return dataType instanceof Array
            ? Long.valueOf(((Array) dataType).getNumElements())
            : null;
    }

    private Long elementByteLen(DataType dataType) {
        return dataType instanceof Array
            ? Long.valueOf(((Array) dataType).getElementLength())
            : null;
    }

    private String dataTypeKind(DataType dataType) {
        if (dataType instanceof Structure) {
            return "structure";
        }
        if (dataType instanceof Union) {
            return "union";
        }
        if (dataType instanceof TypeDef) {
            return "typedef";
        }
        if (dataType instanceof Pointer) {
            return "pointer";
        }
        if (dataType instanceof Array) {
            return "array";
        }
        if (dataType instanceof ghidra.program.model.data.Enum) {
            return "enum";
        }
        return "primitive";
    }

    private String dataTypeId(DataType dataType) {
        if (dataType == null) {
            return null;
        }
        String universal = universalId(dataType);
        String path = firstNonEmpty(dataType.getPathName(), dataType.getDisplayName(), dataType.getName());
        if (universal != null) {
            return "ghidra:datatype:" + nullToToken(path) + ":" + nullToToken(universal);
        }
        return "ghidra:datatype:" + nullToToken(path);
    }

    private String displayName(DataType dataType) {
        return dataType == null ? null : dataType.getDisplayName();
    }

    private String categoryPath(DataType dataType) {
        return dataType == null || dataType.getCategoryPath() == null
            ? null
            : dataType.getCategoryPath().toString();
    }

    private String universalId(DataType dataType) {
        return dataType == null || dataType.getUniversalID() == null
            ? null
            : dataType.getUniversalID().toString();
    }

    private void collectEquates(List<String> equateFacts) {
        EquateTable equateTable = currentProgram.getEquateTable();
        Iterator<Equate> iterator = equateTable.getEquates();
        while (iterator.hasNext()) {
            Equate equate = iterator.next();
            EquateReference[] references = equate.getReferences();
            if (references == null || references.length == 0) {
                continue;
            }
            List<String> referenceFacts = new ArrayList<>();
            for (int i = 0; i < references.length; i++) {
                EquateReference reference = references[i];
                if (reference == null || reference.getAddress() == null) {
                    continue;
                }
                String address = address(reference.getAddress());
                long dynamicHash = reference.getDynamicHashValue();
                referenceFacts.add("      {" +
                    "\"reference_id\": " + json("ghidra:equate_ref:" + equate.getName() + ":" + address + ":" + i) + ", " +
                    "\"address\": " + json(address) + ", " +
                    "\"operand_index\": " + shortOrNull(reference.getOpIndex()) + ", " +
                    "\"dynamic_hash\": " + unsignedLongOrNull(dynamicHash) + ", " +
                    "\"instruction_id\": " + jsonOrNull("ghidra:instruction:" + address) + ", " +
                    "\"statement_id\": null, " +
                    "\"evidence_ids\": [" + json("ghidra:equate_ref:" + address) + "]" +
                    "}");
            }
            if (referenceFacts.isEmpty()) {
                continue;
            }
            String enumUuid = equate.getEnumUUID() == null
                ? null
                : Long.toString(equate.getEnumUUID().getValue());
            String displayValue = equate.getDisplayValue();
            String format = equateFormat(equate.getName(), displayValue);
            equateFacts.add("    {" +
                "\"equate_id\": " + json("ghidra:equate:" + equate.getName()) + ", " +
                "\"function_id\": null, " +
                "\"name\": " + json(equate.getName()) + ", " +
                "\"display_name\": " + json(equate.getDisplayName()) + ", " +
                "\"value\": " + equate.getValue() + ", " +
                "\"display_value\": " + jsonOrNull(displayValue) + ", " +
                "\"format\": " + jsonOrNull(format) + ", " +
                "\"enum_uuid\": " + jsonOrNull(enumUuid) + ", " +
                "\"enum_based\": " + equate.isEnumBased() + ", " +
                "\"valid_uuid\": " + equate.isValidUUID() + ", " +
                "\"references\": [\n" + String.join(",\n", referenceFacts) + "\n    ], " +
                "\"evidence_ids\": [" + json("ghidra:equate:" + equate.getName()) + "]" +
                "}");
        }
    }

    private void collectExternalLinkages(
            List<Function> functions,
            List<String> externalLinkageFacts) {
        ExternalManager externalManager = currentProgram.getExternalManager();
        Set<String> seenLinkages = new HashSet<>();

        for (ExternalLocation location : currentProgram.getSymbolTable().getExternalLocations()) {
            String linkageKey = "location:" + externalLocationKey(location);
            if (seenLinkages.add(linkageKey)) {
                externalLinkageFacts.add(externalLinkageFact(
                    externalManager, location, null, null, null, new ArrayList<>()));
            }
        }

        for (Function function : functions) {
            if (!function.isThunk()) {
                continue;
            }
            Function directTarget = function.getThunkedFunction(false);
            Function recursiveTarget = function.getThunkedFunction(true);
            ExternalLocation location = externalLocationForFunction(recursiveTarget);
            if (location == null) {
                location = externalLocationForFunction(directTarget);
            }
            if (location == null) {
                continue;
            }
            String linkageKey = "thunk:" + address(function.getEntryPoint()) + ":" +
                externalLocationKey(location);
            if (seenLinkages.add(linkageKey)) {
                externalLinkageFacts.add(externalLinkageFact(
                    externalManager,
                    location,
                    function,
                    directTarget,
                    recursiveTarget,
                    thunkChainFacts(function)));
            }
        }
    }

    private String externalLinkageFact(
            ExternalManager externalManager,
            ExternalLocation location,
            Function localThunk,
            Function directTarget,
            Function recursiveTarget,
            List<String> thunkChainFacts) {
        String libraryName = location.getLibraryName();
        Address localAddress = localThunk == null
            ? location.getExternalSpaceAddress()
            : localThunk.getEntryPoint();
        Function externalFunction = location.getFunction();
        DataType dataType = location.getDataType();
        int libraryOrdinal = libraryName == null ? -1 : externalManager.getLibraryOrdinal(libraryName);
        String linkageId = "ghidra:external_linkage:" + address(localAddress) + ":" +
            nullToToken(location.getLabel());
        String evidenceId = localThunk == null
            ? "ghidra:external_location:" + nullToToken(location.getLabel())
            : "ghidra:external_thunk:" + address(localThunk.getEntryPoint());

        return "    {" +
            "\"linkage_id\": " + json(linkageId) + ", " +
            "\"function_id\": " + jsonOrNull(functionId(localThunk)) + ", " +
            "\"local_address\": " + json(address(localAddress)) + ", " +
            "\"thunked_function_id\": " + jsonOrNull(functionId(directTarget)) + ", " +
            "\"thunked_address\": " + jsonOrNull(functionEntry(directTarget)) + ", " +
            "\"recursive_target_function_id\": " + jsonOrNull(functionId(recursiveTarget)) + ", " +
            "\"recursive_target_address\": " + jsonOrNull(functionEntry(recursiveTarget)) + ", " +
            "\"external_library\": " + json(libraryName) + ", " +
            "\"external_library_path\": " + jsonOrNull(location.getExternalLibraryPath()) + ", " +
            "\"library_ordinal\": " + integerOrNull(libraryOrdinal < 0 ? null : libraryOrdinal) + ", " +
            "\"parent_namespace\": " + jsonOrNull(location.getParentName()) + ", " +
            "\"external_label\": " + json(location.getLabel()) + ", " +
            "\"original_imported_name\": " + jsonOrNull(location.getOriginalImportedName()) + ", " +
            "\"external_address\": " + jsonOrNull(address(location.getAddress())) + ", " +
            "\"external_space_address\": " + jsonOrNull(address(location.getExternalSpaceAddress())) + ", " +
            "\"source_type\": " + jsonOrNull(location.getSource().toString()) + ", " +
            "\"is_function\": " + location.isFunction() + ", " +
            "\"data_type\": " + jsonOrNull(dataType == null ? null : dataType.getDisplayName()) + ", " +
            "\"function_signature\": " + jsonOrNull(functionSignature(externalFunction)) + ", " +
            "\"thunk_chain\": " + nestedObjectArray(thunkChainFacts, "    ") + ", " +
            "\"evidence_ids\": [" + json(evidenceId) + "]" +
            "}";
    }

    private List<String> thunkChainFacts(Function thunk) {
        List<String> facts = new ArrayList<>();
        Function cursor = thunk;
        Set<String> seenFunctions = new HashSet<>();
        for (int depth = 0; depth < 32 && cursor != null && cursor.isThunk(); depth++) {
            String cursorEntry = address(cursor.getEntryPoint());
            if (!seenFunctions.add(cursorEntry)) {
                break;
            }
            Function target = cursor.getThunkedFunction(false);
            boolean terminal = target == null || !target.isThunk();
            facts.add("      {" +
                "\"link_id\": " + json("ghidra:external_thunk:" + cursorEntry + ":" + depth) + ", " +
                "\"function_id\": " + json(functionId(cursor)) + ", " +
                "\"address\": " + json(cursorEntry) + ", " +
                "\"target_function_id\": " + jsonOrNull(functionId(target)) + ", " +
                "\"target_address\": " + jsonOrNull(functionEntry(target)) + ", " +
                "\"recursive_depth\": " + depth + ", " +
                "\"is_terminal\": " + terminal + ", " +
                "\"evidence_ids\": [" + json("ghidra:thunk_reference:" + cursorEntry) + "]" +
                "}");
            cursor = target;
        }
        return facts;
    }

    private ExternalLocation externalLocationForFunction(Function function) {
        if (function == null || !function.isExternal()) {
            return null;
        }
        return function.getExternalLocation();
    }

    private String externalLocationKey(ExternalLocation location) {
        return nullToToken(location.getLibraryName()) + ":" +
            nullToToken(location.getLabel()) + ":" +
            nullToToken(location.getOriginalImportedName()) + ":" +
            nullToToken(address(location.getExternalSpaceAddress()));
    }

    private String functionId(Function function) {
        String entry = functionEntry(function);
        return entry == null ? null : "ghidra:function:" + entry;
    }

    private String functionEntry(Function function) {
        return function == null ? null : address(function.getEntryPoint());
    }

    private String functionSignature(Function function) {
        return function == null ? null : function.getPrototypeString(false, true);
    }

    private void collectFunctionPrototypes(List<Function> functions, List<String> facts)
            throws Exception {
        for (Function function : functions) {
            monitor.checkCancelled();
            facts.add(functionPrototypeFact(function));
        }
    }

    private String functionPrototypeFact(Function function) {
        String entry = functionEntry(function);
        Parameter returnParameter = function.getReturn();
        DataType returnType = returnParameter == null ? function.getReturnType()
            : returnParameter.getDataType();
        Function thunkedFunction = function.isThunk()
            ? function.getThunkedFunction(false)
            : null;
        Integer stackPurgeSize = function.isStackPurgeSizeValid()
            ? function.getStackPurgeSize()
            : null;
        String prototypeId = "ghidra:prototype:" + entry;
        return "    {" +
            "\"prototype_id\": " + json(prototypeId) + ", " +
            "\"function_id\": " + jsonOrNull(functionId(function)) + ", " +
            "\"entry_point\": " + jsonOrNull(entry) + ", " +
            "\"name\": " + jsonOrNull(function.getName()) + ", " +
            "\"prototype\": " + jsonOrNull(function.getPrototypeString(false, true)) + ", " +
            "\"calling_convention\": " + jsonOrNull(function.getCallingConventionName()) + ", " +
            "\"return_type\": " + jsonOrNull(displayName(returnType)) + ", " +
            "\"return_type_id\": " + jsonOrNull(dataTypeId(returnType)) + ", " +
            "\"return_type_kind\": " + jsonOrNull(dataTypeKind(returnType)) + ", " +
            "\"return_type_byte_len\": " + nonNegativeIntegerOrNull(
                returnType == null ? -1 : returnType.getLength()) + ", " +
            "\"return_storage\": " + variableStorageFactOrUnassigned(
                returnParameter == null ? null : returnParameter.getVariableStorage(), false) + ", " +
            "\"parameters\": " + nestedObjectArray(functionPrototypeParameterFacts(function), "    ") + ", " +
            "\"has_varargs\": " + function.hasVarArgs() + ", " +
            "\"has_no_return\": " + function.hasNoReturn() + ", " +
            "\"is_inline\": " + function.isInline() + ", " +
            "\"is_thunk\": " + function.isThunk() + ", " +
            "\"thunked_function_id\": " + jsonOrNull(functionId(thunkedFunction)) + ", " +
            "\"thunked_entry_point\": " + jsonOrNull(functionEntry(thunkedFunction)) + ", " +
            "\"has_custom_storage\": " + function.hasCustomVariableStorage() + ", " +
            "\"stack_purge_size\": " + integerOrNull(stackPurgeSize) + ", " +
            "\"signature_source\": " + jsonOrNull(
                function.getSignatureSource() == null
                    ? null
                    : function.getSignatureSource().toString()) + ", " +
            "\"evidence_ids\": [" + json(prototypeId) + "]" +
            "}";
    }

    private List<String> functionPrototypeParameterFacts(Function function) {
        List<String> facts = new ArrayList<>();
        Parameter[] parameters = function.getParameters();
        if (parameters == null) {
            return facts;
        }
        String entry = functionEntry(function);
        for (int i = 0; i < parameters.length; i++) {
            Parameter parameter = parameters[i];
            if (parameter == null) {
                continue;
            }
            DataType dataType = parameter.getDataType();
            DataType formalDataType = parameter.getFormalDataType();
            String parameterId = "ghidra:prototype_param:" + entry + ":" +
                parameter.getOrdinal() + ":" + nullToToken(parameter.getName());
            facts.add("      {" +
                "\"ordinal\": " + parameter.getOrdinal() + ", " +
                "\"name\": " + jsonOrNull(parameter.getName()) + ", " +
                "\"data_type\": " + jsonOrNull(displayName(dataType)) + ", " +
                "\"data_type_id\": " + jsonOrNull(dataTypeId(dataType)) + ", " +
                "\"data_type_kind\": " + jsonOrNull(dataTypeKind(dataType)) + ", " +
                "\"data_type_byte_len\": " + nonNegativeIntegerOrNull(
                    dataType == null ? -1 : dataType.getLength()) + ", " +
                "\"storage\": " + variableStorageFactOrUnassigned(
                    parameter.getVariableStorage(), false) + ", " +
                "\"auto_parameter\": " + parameter.isAutoParameter() + ", " +
                "\"auto_parameter_type\": " + jsonOrNull(
                    parameter.getAutoParameterType() == null
                        ? null
                        : parameter.getAutoParameterType().name()) + ", " +
                "\"forced_indirect\": " + parameter.isForcedIndirect() + ", " +
                "\"formal_data_type\": " + jsonOrNull(displayName(formalDataType)) + ", " +
                "\"formal_data_type_id\": " + jsonOrNull(dataTypeId(formalDataType)) + ", " +
                "\"formal_data_type_kind\": " + jsonOrNull(dataTypeKind(formalDataType)) + ", " +
                "\"formal_data_type_byte_len\": " + nonNegativeIntegerOrNull(
                    formalDataType == null ? -1 : formalDataType.getLength()) + ", " +
                "\"comment\": " + jsonOrNull(parameter.getComment()) + ", " +
                "\"source_type\": " + jsonOrNull(
                    parameter.getSource() == null ? null : parameter.getSource().toString()) + ", " +
                "\"evidence_ids\": [" + json(parameterId) + "]" +
                "}");
        }
        return facts;
    }

    private void collectHighVariables(
            List<Function> functions,
            int maxFunctions,
            int timeoutSeconds,
            List<String> highVariableFacts) throws Exception {
        if (maxFunctions <= 0) {
            return;
        }

        DecompInterface decompiler = new DecompInterface();
        try {
            DecompileOptions options = new DecompileOptions();
            options.setDefaultTimeout(timeoutSeconds);
            decompiler.setOptions(options);
            decompiler.toggleCCode(false);
            decompiler.toggleSyntaxTree(true);
            if (!decompiler.openProgram(currentProgram)) {
                return;
            }
            int decompiled = 0;
            Set<String> seenHighVariables = new HashSet<>();
            for (Function function : functions) {
                monitor.checkCancelled();
                if (decompiled >= maxFunctions) {
                    break;
                }
                decompiled++;
                DecompileResults result =
                    decompiler.decompileFunction(function, timeoutSeconds, monitor);
                if (!result.decompileCompleted() || result.isTimedOut() || result.failedToStart() ||
                    !result.isValid() || result.getHighFunction() == null) {
                    decompiler.flushCache();
                    continue;
                }
                HighFunction highFunction = result.getHighFunction();
                LocalSymbolMap locals = highFunction.getLocalSymbolMap();
                if (locals != null) {
                    collectHighVariablesFromSymbols(
                        function,
                        locals.getSymbols(),
                        false,
                        seenHighVariables,
                        highVariableFacts);
                }
                GlobalSymbolMap globals = highFunction.getGlobalSymbolMap();
                if (globals != null) {
                    collectHighVariablesFromSymbols(
                        function,
                        globals.getSymbols(),
                        true,
                        seenHighVariables,
                        highVariableFacts);
                }
                decompiler.flushCache();
            }
        }
        finally {
            decompiler.dispose();
        }
    }

    private void collectHighVariablesFromSymbols(
            Function function,
            Iterator<HighSymbol> symbols,
            boolean global,
            Set<String> seenHighVariables,
            List<String> highVariableFacts) throws Exception {
        while (symbols.hasNext()) {
            monitor.checkCancelled();
            HighSymbol symbol = symbols.next();
            if (symbol == null) {
                continue;
            }
            HighVariable variable = symbol.getHighVariable();
            if (variable == null) {
                continue;
            }
            String fact = highVariableFact(function, symbol, variable, global);
            if (fact == null) {
                continue;
            }
            String key = address(function.getEntryPoint()) + ":" + highVariableId(function, symbol, variable, global);
            if (seenHighVariables.add(key)) {
                highVariableFacts.add(fact);
            }
        }
    }

    private String highVariableFact(
            Function function,
            HighSymbol symbol,
            HighVariable variable,
            boolean global) {
        String entry = address(function.getEntryPoint());
        String variableId = highVariableId(function, symbol, variable, global);
        String symbolId = symbol.getId() == 0
            ? null
            : "ghidra:symbol:" + Long.toHexString(symbol.getId());
        DataType dataType = variable.getDataType() == null
            ? symbol.getDataType()
            : variable.getDataType();
        String kind = highVariableKind(symbol, variable, global);
        Address firstUseAddress = symbol.getPCAddress();
        Integer categoryIndex = symbol.getCategoryIndex() >= 0
            ? symbol.getCategoryIndex()
            : null;
        Integer highSymbolOffset = variable.getOffset() >= 0
            ? variable.getOffset()
            : null;

        return "    {" +
            "\"variable_id\": " + json(variableId) + ", " +
            "\"function_id\": " + json(functionId(function)) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"symbol_id\": " + jsonOrNull(symbolId) + ", " +
            "\"name\": " + json(variable.getName()) + ", " +
            "\"kind\": " + json(kind) + ", " +
            "\"category_index\": " + integerOrNull(categoryIndex) + ", " +
            "\"data_type\": " + json(displayName(dataType)) + ", " +
            "\"data_type_id\": " + jsonOrNull(dataTypeId(dataType)) + ", " +
            "\"storage\": " + highVariableStorageFact(symbol.getStorage(), variable) + ", " +
            "\"first_use_address\": " + jsonOrNull(address(firstUseAddress)) + ", " +
            "\"first_use_offset\": " + longOrNull(firstUseOffset(function, firstUseAddress)) + ", " +
            "\"name_locked\": " + symbol.isNameLocked() + ", " +
            "\"type_locked\": " + symbol.isTypeLocked() + ", " +
            "\"isolated\": " + symbol.isIsolated() + ", " +
            "\"is_this_pointer\": " + symbol.isThisPointer() + ", " +
            "\"is_hidden_return\": " + symbol.isHiddenReturn() + ", " +
            "\"mutability\": \"normal\", " +
            "\"high_symbol_offset\": " + integerOrNull(highSymbolOffset) + ", " +
            "\"instances\": " + nestedObjectArray(
                highVariableInstanceFacts(variableId, variable), "    ") + ", " +
            "\"evidence_ids\": [" + json("ghidra:highvar:" + entry + ":" + nullToToken(variable.getName())) + "]" +
            "}";
    }

    private String highVariableId(
            Function function,
            HighSymbol symbol,
            HighVariable variable,
            boolean global) {
        return "ghidra:highvar:" +
            address(function.getEntryPoint()) + ":" +
            highVariableKind(symbol, variable, global) + ":" +
            nullToToken(symbol.getId() == 0 ? variable.getName() : Long.toHexString(symbol.getId())) +
            ":" + nullToToken(variable.getName());
    }

    private String highVariableKind(HighSymbol symbol, HighVariable variable, boolean global) {
        if (symbol.isHiddenReturn()) {
            return "return_storage";
        }
        if (symbol.isParameter()) {
            return "parameter";
        }
        if (global) {
            return "global";
        }
        VariableStorage storage = symbol.getStorage();
        if (storage != null && storage.isHashStorage()) {
            return "temporary";
        }
        return "local";
    }

    private Long firstUseOffset(Function function, Address firstUseAddress) {
        if (function == null || firstUseAddress == null || firstUseAddress == Address.NO_ADDRESS) {
            return null;
        }
        try {
            return firstUseAddress.subtract(function.getEntryPoint());
        }
        catch (RuntimeException e) {
            return null;
        }
    }

    private String highVariableStorageFact(VariableStorage storage, HighVariable variable) {
        if (storage != null) {
            return variableStorageFact(storage, variable.requiresDynamicStorage());
        }
        Varnode representative = variable.getRepresentative();
        if (representative != null) {
            return varnodeStorageFact(representative, variable.requiresDynamicStorage());
        }
        return "{\"storage\": \"unassigned\", \"kind\": \"unassigned\", \"byte_len\": 0, " +
            "\"pieces\": [], \"dynamic_storage_required\": true, \"forced_indirect\": false, " +
            "\"auto_parameter_type\": null}";
    }

    private List<String> highVariableInstanceFacts(String variableId, HighVariable variable) {
        List<String> facts = new ArrayList<>();
        Varnode representative = variable.getRepresentative();
        Varnode[] instances = variable.getInstances();
        if (instances == null || instances.length == 0) {
            instances = representative == null ? new Varnode[0] : new Varnode[] { representative };
        }
        for (int i = 0; i < instances.length; i++) {
            Varnode instance = instances[i];
            if (instance == null) {
                continue;
            }
            String instanceId = variableId + ":instance:" + i + ":" + nullToToken(varnode(instance));
            facts.add("      {" +
                "\"instance_id\": " + json(instanceId) + ", " +
                "\"storage\": " + varnodeStorageFact(instance, false) + ", " +
                "\"pc_address\": " + jsonOrNull(varnodePcAddress(instance)) + ", " +
                "\"defining_pcode_id\": " + jsonOrNull(varnodeDefiningPcodeId(instance)) + ", " +
                "\"merge_group\": " + integerOrNull((int) instance.getMergeGroup()) + ", " +
                "\"is_representative\": " + instance.equals(representative) + ", " +
                "\"evidence_ids\": [" + json(instanceId) + "]" +
                "}");
        }
        return facts;
    }

    private void collectStackFrames(List<Function> functions, List<String> stackFrameFacts)
            throws Exception {
        for (Function function : functions) {
            monitor.checkCancelled();
            StackFrame frame = function.getStackFrame();
            if (frame == null) {
                continue;
            }
            String fact = stackFrameFact(function, frame);
            if (fact != null) {
                stackFrameFacts.add(fact);
            }
        }
    }

    private String stackFrameFact(Function function, StackFrame frame) {
        String entry = address(function.getEntryPoint());
        Register stackPointer = currentProgram.getCompilerSpec().getStackPointer();
        String frameId = "ghidra:stack_frame:" + entry;
        return "    {" +
            "\"frame_id\": " + json(frameId) + ", " +
            "\"function_id\": " + json(functionId(function)) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"frame_size\": " + nonNegative(frame.getFrameSize()) + ", " +
            "\"local_size\": " + nonNegative(frame.getLocalSize()) + ", " +
            "\"parameter_size\": " + nonNegative(frame.getParameterSize()) + ", " +
            "\"parameter_offset\": " + integerOrNull(
                frame.getParameterOffset() == StackFrame.UNKNOWN_PARAM_OFFSET
                    ? null
                    : frame.getParameterOffset()) + ", " +
            "\"return_address_offset\": " + integerOrNull(frame.getReturnAddressOffset()) + ", " +
            "\"growth\": " + json(frame.growsNegative() ? "negative" : "positive") + ", " +
            "\"stack_pointer_register\": " + jsonOrNull(
                stackPointer == null ? null : stackPointer.getName()) + ", " +
            "\"custom_variable_storage\": " + function.hasCustomVariableStorage() + ", " +
            "\"variables\": " + nestedObjectArray(stackFrameVariableFacts(function, frame), "    ") + ", " +
            "\"evidence_ids\": [" + json(frameId) + "]" +
            "}";
    }

    private List<String> stackFrameVariableFacts(Function function, StackFrame frame) {
        List<String> facts = new ArrayList<>();
        Variable[] variables = frame.getStackVariables();
        if (variables == null) {
            return facts;
        }
        for (int i = 0; i < variables.length; i++) {
            Variable variable = variables[i];
            if (variable == null) {
                continue;
            }
            facts.add(stackFrameVariableFact(function, frame, variable, i));
        }
        return facts;
    }

    private String stackFrameVariableFact(
            Function function,
            StackFrame frame,
            Variable variable,
            int index) {
        int offset = stackVariableOffset(variable);
        DataType dataType = variable.getDataType();
        String variableId = "ghidra:stack_var:" + address(function.getEntryPoint()) + ":" +
            offset + ":" + nullToToken(variable.getName());
        Integer ordinal = variable instanceof Parameter
            ? ((Parameter) variable).getOrdinal()
            : null;
        return "      {" +
            "\"variable_id\": " + json(variableId) + ", " +
            "\"name\": " + json(variable.getName()) + ", " +
            "\"kind\": " + json(stackFrameVariableKind(frame, variable, offset)) + ", " +
            "\"offset\": " + offset + ", " +
            "\"byte_len\": " + nonNegative(variable.getLength()) + ", " +
            "\"ordinal\": " + integerOrNull(ordinal) + ", " +
            "\"data_type\": " + jsonOrNull(displayName(dataType)) + ", " +
            "\"data_type_id\": " + jsonOrNull(dataTypeId(dataType)) + ", " +
            "\"storage\": " + stackVariableStorageFact(variable) + ", " +
            "\"source_type\": " + jsonOrNull(
                variable.getSource() == null ? null : variable.getSource().toString()) + ", " +
            "\"high_variable_id\": null, " +
            "\"name_locked\": false, " +
            "\"type_locked\": false, " +
            "\"evidence_ids\": [" + json(variableId + ":" + index) + "]" +
            "}";
    }

    private int stackVariableOffset(Variable variable) {
        if (variable != null && variable.isStackVariable()) {
            try {
                return variable.getStackOffset();
            }
            catch (RuntimeException e) {
                // Fall back to the first storage varnode below.
            }
        }
        Varnode first = variable == null ? null : variable.getFirstStorageVarnode();
        return first == null ? 0 : (int) first.getOffset();
    }

    private String stackFrameVariableKind(StackFrame frame, Variable variable, int offset) {
        if (frame != null && offset == frame.getReturnAddressOffset()) {
            return "return_address";
        }
        if (variable instanceof Parameter || (frame != null && frame.isParameterOffset(offset))) {
            return "parameter";
        }
        return "local";
    }

    private String stackVariableStorageFact(Variable variable) {
        VariableStorage storage = variable == null ? null : variable.getVariableStorage();
        if (storage != null) {
            return variableStorageFact(storage, false);
        }
        Varnode first = variable == null ? null : variable.getFirstStorageVarnode();
        if (first != null) {
            return varnodeStorageFact(first, false);
        }
        return "{\"storage\": \"unassigned\", \"kind\": \"unassigned\", \"byte_len\": 0, " +
            "\"pieces\": [], \"dynamic_storage_required\": false, \"forced_indirect\": false, " +
            "\"auto_parameter_type\": null}";
    }

    private void collectParameterMeasures(
            List<Function> functions,
            int maxFunctions,
            int timeoutSeconds,
            List<String> facts)
            throws Exception {
        DecompInterface decompiler = new DecompInterface();
        DecompileOptions options = new DecompileOptions();
        decompiler.setOptions(options);
        decompiler.toggleCCode(false);
        decompiler.toggleSyntaxTree(false);
        decompiler.toggleParamMeasures(true);
        decompiler.setSimplificationStyle("paramid");
        if (!decompiler.openProgram(currentProgram)) {
            return;
        }
        try {
            int count = 0;
            for (Function function : functions) {
                monitor.checkCancelled();
                if (count++ >= maxFunctions) {
                    break;
                }
                DecompileResults result =
                    decompiler.decompileFunction(function, timeoutSeconds, monitor);
                if (result == null || !result.decompileCompleted()) {
                    continue;
                }
                HighParamID paramId = result.getHighParamID();
                if (paramId == null) {
                    continue;
                }
                for (int i = 0; i < paramId.getNumInputs(); i++) {
                    ParamMeasure measure = paramId.getInput(i);
                    if (measure != null) {
                        facts.add(parameterMeasureFact(function, paramId, measure, "input", i, true));
                    }
                }
                for (int i = 0; i < paramId.getNumOutputs(); i++) {
                    ParamMeasure measure = paramId.getOutput(i);
                    if (measure != null) {
                        facts.add(parameterMeasureFact(function, paramId, measure, "output", i, true));
                    }
                }
            }
        }
        finally {
            decompiler.dispose();
        }
    }

    private String parameterMeasureFact(
            Function function,
            HighParamID paramId,
            ParamMeasure measure,
            String io,
            int index,
            boolean justPrototype) {
        String entry = address(function.getEntryPoint());
        Varnode measureVarnode = measure.getVarnode();
        String storage = nullToToken(varnode(measureVarnode));
        String measureId =
            "ghidra:param_measure:" + entry + ":" + io + ":" + index + ":" + storage;
        DataType dataType = measure.getDataType();
        Integer extraPop = paramId.getProtoExtraPop() == PrototypeModel.UNKNOWN_EXTRAPOP
            ? null
            : paramId.getProtoExtraPop();
        return "    {" +
            "\"measure_id\": " + json(measureId) + ", " +
            "\"function_id\": " + json(functionId(function)) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"io\": " + json(io) + ", " +
            "\"rank\": " + json(parameterMeasureRank(io, measure.getRank())) + ", " +
            "\"rank_value\": " + integerOrNull(measure.getRank()) + ", " +
            "\"storage\": " + varnodeStorageFact(measureVarnode, false) + ", " +
            "\"data_type\": " + jsonOrNull(displayName(dataType)) + ", " +
            "\"data_type_id\": " + jsonOrNull(dataTypeId(dataType)) + ", " +
            "\"model_name\": " + jsonOrNull(paramId.getModelName()) + ", " +
            "\"extra_pop\": " + integerOrNull(extraPop) + ", " +
            "\"just_prototype\": " + justPrototype + ", " +
            "\"base_variable_id\": null, " +
            "\"source_statement_id\": null, " +
            "\"num_calls\": 0, " +
            "\"evidence_ids\": [" + json(measureId) + "]" +
            "}";
    }

    private String parameterMeasureRank(String io, int rank) {
        if ("input".equals(io)) {
            switch (rank) {
                case 2:
                    return "direct_read";
                case 4:
                    return "sub_function_parameter";
                case 5:
                    return "this_function_return";
                case 6:
                    return "indirect";
                case 7:
                    return "worst_rank";
                default:
                    return "unknown";
            }
        }
        switch (rank) {
            case 1:
                return "direct_write_without_read";
            case 2:
                return "direct_write_with_read";
            case 3:
                return "direct_write_unknown_read";
            case 4:
                return "this_function_parameter";
            case 5:
                return "sub_function_return";
            case 6:
                return "indirect";
            case 7:
                return "worst_rank";
            default:
                return "unknown";
        }
    }

    private void collectCallStackEffects(
            List<Function> functions,
            Listing listing,
            List<String> callStackEffectFacts) throws Exception {
        Register stackPointer = currentProgram.getCompilerSpec().getStackPointer();
        AddressSpace stackSpace = currentProgram.getCompilerSpec().getStackSpace();
        for (Function function : functions) {
            monitor.checkCancelled();
            CallDepthChangeInfo depthInfo =
                new CallDepthChangeInfo(function, function.getBody(), stackPointer, monitor);
            for (Instruction instruction : listing.getInstructions(function.getBody(), true)) {
                monitor.checkCancelled();
                if (!instruction.getFlowType().isCall()) {
                    continue;
                }
                callStackEffectFacts.add(callStackEffectFact(
                    function,
                    instruction,
                    depthInfo,
                    stackPointer,
                    stackSpace));
            }
        }
    }

    private String callStackEffectFact(
            Function function,
            Instruction instruction,
            CallDepthChangeInfo depthInfo,
            Register stackPointer,
            AddressSpace stackSpace) {
        String entry = address(function.getEntryPoint());
        String callsite = address(instruction.getMinAddress());
        Function callee = calleeFunctionForCall(instruction);
        PrototypeModel model = callee == null ? null : callee.getCallingConvention();
        Integer stackOffsetBeforeCall = knownStackDepth(depthInfo.getDepth(instruction.getMinAddress()));
        Integer instructionStackDepthChange =
            knownStackDepth(depthInfo.getInstructionStackDepthChange(instruction));
        Integer effectiveExtraPop = knownStackDepth(depthInfo.getCallChange(instruction.getMinAddress()));
        Integer purgeSize = callee != null && callee.isStackPurgeSizeValid()
            ? knownStackDepth(callee.getStackPurgeSize())
            : null;
        Integer extraPop = model != null && model.getExtrapop() != PrototypeModel.UNKNOWN_EXTRAPOP
            ? model.getExtrapop()
            : null;
        Integer stackShift = model != null && model.getStackshift() >= 0
            ? model.getStackshift()
            : null;
        String effectId = "ghidra:stack_effect:" + entry + ":" + callsite;
        List<String> warnings = callStackEffectWarnings(
            callee,
            stackOffsetBeforeCall,
            effectiveExtraPop,
            purgeSize,
            extraPop);
        return "    {" +
            "\"effect_id\": " + json(effectId) + ", " +
            "\"function_id\": " + json(functionId(function)) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"callsite_address\": " + json(callsite) + ", " +
            "\"callee_function_id\": " + jsonOrNull(functionId(callee)) + ", " +
            "\"callee_name\": " + jsonOrNull(callee == null ? null : callee.getName()) + ", " +
            "\"call_opcode\": " + jsonOrNull(instruction.getMnemonicString()) + ", " +
            "\"prototype_model\": " + jsonOrNull(model == null ? null : model.getName()) + ", " +
            "\"stack_pointer_register\": " + jsonOrNull(
                stackPointer == null ? null : stackPointer.getName()) + ", " +
            "\"stack_space\": " + jsonOrNull(
                stackSpace == null ? null : stackSpace.getName()) + ", " +
            "\"stack_offset_before_call\": " + integerOrNull(stackOffsetBeforeCall) + ", " +
            "\"instruction_stack_depth_change\": " +
                integerOrNull(instructionStackDepthChange) + ", " +
            "\"stack_shift_bytes\": " + integerOrNull(stackShift) + ", " +
            "\"purge_size_bytes\": " + integerOrNull(purgeSize) + ", " +
            "\"extra_pop_bytes\": " + integerOrNull(extraPop) + ", " +
            "\"effective_extra_pop_bytes\": " + integerOrNull(effectiveExtraPop) + ", " +
            "\"companion_solution_bytes\": " + integerOrNull(0) + ", " +
            "\"solver_variable_count\": " +
                solverVariableCount(stackOffsetBeforeCall, effectiveExtraPop, purgeSize, extraPop) + ", " +
            "\"missed_variable_count\": " + warnings.size() + ", " +
            "\"status\": " + json(callStackEffectStatus(effectiveExtraPop, purgeSize, extraPop)) + ", " +
            "\"warnings\": " + stringArray(warnings) + ", " +
            "\"evidence_ids\": [" + json(effectId) + "]" +
            "}";
    }

    private Function calleeFunctionForCall(Instruction instruction) {
        if (instruction == null) {
            return null;
        }
        for (Reference reference : instruction.getReferencesFrom()) {
            if (reference.getReferenceType().isCall()) {
                Function callee =
                    currentProgram.getFunctionManager().getFunctionAt(reference.getToAddress());
                if (callee != null) {
                    return callee;
                }
                callee =
                    currentProgram.getFunctionManager().getFunctionContaining(reference.getToAddress());
                if (callee != null) {
                    return callee;
                }
            }
        }
        return null;
    }

    private Integer knownStackDepth(int value) {
        if (value == Function.UNKNOWN_STACK_DEPTH_CHANGE ||
                value == Function.INVALID_STACK_DEPTH_CHANGE) {
            return null;
        }
        return value;
    }

    private int solverVariableCount(
            Integer stackOffsetBeforeCall,
            Integer effectiveExtraPop,
            Integer purgeSize,
            Integer extraPop) {
        int count = 0;
        count += stackOffsetBeforeCall == null ? 0 : 1;
        count += effectiveExtraPop == null ? 0 : 1;
        count += purgeSize == null ? 0 : 1;
        count += extraPop == null ? 0 : 1;
        return count;
    }

    private String callStackEffectStatus(
            Integer effectiveExtraPop,
            Integer purgeSize,
            Integer extraPop) {
        if (effectiveExtraPop != null) {
            return "solved";
        }
        if (purgeSize != null || extraPop != null) {
            return "known";
        }
        return "unknown";
    }

    private List<String> callStackEffectWarnings(
            Function callee,
            Integer stackOffsetBeforeCall,
            Integer effectiveExtraPop,
            Integer purgeSize,
            Integer extraPop) {
        List<String> warnings = new ArrayList<>();
        if (callee == null) {
            warnings.add("callee_unresolved");
        }
        if (stackOffsetBeforeCall == null) {
            warnings.add("stack_offset_before_call_unknown");
        }
        if (effectiveExtraPop == null) {
            warnings.add("effective_extra_pop_unknown");
        }
        if (purgeSize == null) {
            warnings.add("callee_purge_unknown");
        }
        if (extraPop == null) {
            warnings.add("prototype_extra_pop_unknown");
        }
        return warnings;
    }

    private String variableStorageFact(VariableStorage storage, boolean dynamicStorageRequired) {
        Varnode[] varnodes = storage.getVarnodes();
        return "{\"storage\": " + json(storage.getSerializationString()) + ", " +
            "\"kind\": " + json(variableStorageKind(storage)) + ", " +
            "\"byte_len\": " + storage.size() + ", " +
            "\"pieces\": " + nestedObjectArray(variableStoragePieceFacts(varnodes), "    ") + ", " +
            "\"dynamic_storage_required\": " + dynamicStorageRequired + ", " +
            "\"forced_indirect\": " + storage.isForcedIndirect() + ", " +
            "\"auto_parameter_type\": " + jsonOrNull(
                storage.getAutoParameterType() == null
                    ? null
                    : storage.getAutoParameterType().name()) +
            "}";
    }

    private String variableStorageFactOrUnassigned(
            VariableStorage storage,
            boolean dynamicStorageRequired) {
        if (storage == null) {
            return "{\"storage\": \"unassigned\", \"kind\": \"unassigned\", \"byte_len\": 0, " +
                "\"pieces\": [], \"dynamic_storage_required\": " + dynamicStorageRequired + ", " +
                "\"forced_indirect\": false, \"auto_parameter_type\": null}";
        }
        return variableStorageFact(storage, dynamicStorageRequired);
    }

    private String varnodeStorageFact(Varnode varnode, boolean dynamicStorageRequired) {
        return "{\"storage\": " + json(varnode(varnode)) + ", " +
            "\"kind\": " + json(varnodeStorageKind(varnode)) + ", " +
            "\"byte_len\": " + varnode.getSize() + ", " +
            "\"pieces\": " + nestedObjectArray(
                variableStoragePieceFacts(new Varnode[] { varnode }), "    ") + ", " +
            "\"dynamic_storage_required\": " + dynamicStorageRequired + ", " +
            "\"forced_indirect\": false, " +
            "\"auto_parameter_type\": null}";
    }

    private List<String> variableStoragePieceFacts(Varnode[] varnodes) {
        List<String> pieces = new ArrayList<>();
        if (varnodes == null) {
            return pieces;
        }
        for (int i = 0; i < varnodes.length; i++) {
            Varnode varnode = varnodes[i];
            if (varnode == null || varnode.getAddress() == null) {
                continue;
            }
            Address address = varnode.getAddress();
            String space = address.getAddressSpace() == null
                ? "unknown"
                : address.getAddressSpace().getName();
            String registerName = registerName(varnode);
            String pieceId = "ghidra:storage_piece:" + i + ":" + nullToToken(varnode(varnode));
            pieces.add("      {" +
                "\"piece_id\": " + json(pieceId) + ", " +
                "\"space\": " + json(space) + ", " +
                "\"offset\": " + address.getOffset() + ", " +
                "\"byte_len\": " + varnode.getSize() + ", " +
                "\"register\": " + jsonOrNull(registerName) + ", " +
                "\"is_input\": " + varnode.isInput() + ", " +
                "\"is_addr_tied\": " + varnode.isAddrTied() + ", " +
                "\"is_persistent\": " + varnode.isPersistent() + ", " +
                "\"is_unique\": " + varnode.isUnique() + ", " +
                "\"is_constant\": " + varnode.isConstant() + ", " +
                "\"is_hash\": " + varnode.isHash() + ", " +
                "\"is_stack\": " + address.isStackAddress() +
                "}");
        }
        return pieces;
    }

    private String variableStorageKind(VariableStorage storage) {
        if (storage.isBadStorage()) {
            return "bad";
        }
        if (storage.isVoidStorage()) {
            return "void";
        }
        if (storage.isUnassignedStorage()) {
            return "unassigned";
        }
        if (storage.isCompoundStorage()) {
            return "compound";
        }
        if (storage.isRegisterStorage()) {
            return "register";
        }
        if (storage.isStackStorage()) {
            return "stack";
        }
        if (storage.isMemoryStorage()) {
            return "memory";
        }
        if (storage.isConstantStorage()) {
            return "constant";
        }
        if (storage.isHashStorage()) {
            return "hash";
        }
        if (storage.isUniqueStorage()) {
            return "unique";
        }
        return "unknown";
    }

    private String varnodeStorageKind(Varnode varnode) {
        if (varnode.isRegister()) {
            return "register";
        }
        if (varnode.getAddress() != null && varnode.getAddress().isStackAddress()) {
            return "stack";
        }
        if (varnode.isConstant()) {
            return "constant";
        }
        if (varnode.isHash()) {
            return "hash";
        }
        if (varnode.isUnique()) {
            return "unique";
        }
        if (varnode.getAddress() != null && varnode.getAddress().isMemoryAddress()) {
            return "memory";
        }
        return "unknown";
    }

    private String registerName(Varnode varnode) {
        if (varnode == null || varnode.getAddress() == null || !varnode.isRegister()) {
            return null;
        }
        Register register = currentProgram.getRegister(varnode.getAddress(), varnode.getSize());
        return register == null ? null : register.getName();
    }

    private String varnodePcAddress(Varnode varnode) {
        try {
            return address(varnode.getPCAddress());
        }
        catch (RuntimeException e) {
            return null;
        }
    }

    private String varnodeDefiningPcodeId(Varnode varnode) {
        PcodeOp def = varnode.getDef();
        if (def == null || def.getSeqnum() == null) {
            return null;
        }
        String target = address(def.getSeqnum().getTarget());
        return target == null
            ? null
            : "ghidra:pcode:" + target + ":" + def.getSeqnum().getTime();
    }

    private void collectJumpTables(
            List<Function> functions,
            int maxFunctions,
            int timeoutSeconds,
            List<String> jumpTableFacts) throws Exception {
        if (maxFunctions <= 0) {
            return;
        }

        DecompInterface decompiler = new DecompInterface();
        try {
            DecompileOptions options = new DecompileOptions();
            options.setDefaultTimeout(timeoutSeconds);
            decompiler.setOptions(options);
            decompiler.toggleCCode(false);
            decompiler.toggleSyntaxTree(true);
            if (!decompiler.openProgram(currentProgram)) {
                return;
            }
            int decompiled = 0;
            Set<String> seenJumpTables = new HashSet<>();
            for (Function function : functions) {
                monitor.checkCancelled();
                if (decompiled >= maxFunctions) {
                    break;
                }
                decompiled++;
                DecompileResults result =
                    decompiler.decompileFunction(function, timeoutSeconds, monitor);
                if (!result.decompileCompleted() || !result.isValid() ||
                    result.getHighFunction() == null) {
                    decompiler.flushCache();
                    continue;
                }
                HighFunction highFunction = result.getHighFunction();
                for (JumpTable table : highFunction.getJumpTables()) {
                    monitor.checkCancelled();
                    String switchAddress = address(table.getSwitchAddress());
                    if (switchAddress == null || switchAddress.isEmpty()) {
                        continue;
                    }
                    String tableKey = address(function.getEntryPoint()) + ":" + switchAddress;
                    if (seenJumpTables.add(tableKey)) {
                        String fact = jumpTableFact(function, table);
                        if (fact != null) {
                            jumpTableFacts.add(fact);
                        }
                    }
                }
                decompiler.flushCache();
            }
        }
        finally {
            decompiler.dispose();
        }
    }

    private void collectStructureFieldAccesses(
            List<Function> functions,
            int maxFunctions,
            int timeoutSeconds,
            List<String> structureFieldAccessFacts) throws Exception {
        if (maxFunctions <= 0) {
            return;
        }

        DecompInterface decompiler = new DecompInterface();
        try {
            DecompileOptions options = new DecompileOptions();
            options.setDefaultTimeout(timeoutSeconds);
            decompiler.setOptions(options);
            decompiler.toggleCCode(true);
            decompiler.toggleSyntaxTree(true);
            if (!decompiler.openProgram(currentProgram)) {
                return;
            }
            int decompiled = 0;
            Set<String> seenAccesses = new HashSet<>();
            for (Function function : functions) {
                monitor.checkCancelled();
                if (decompiled >= maxFunctions) {
                    break;
                }
                decompiled++;
                DecompileResults result =
                    decompiler.decompileFunction(function, timeoutSeconds, monitor);
                if (!result.decompileCompleted() || !result.isValid() ||
                    result.getHighFunction() == null || result.getCCodeMarkup() == null) {
                    decompiler.flushCache();
                    continue;
                }

                HighFunction highFunction = result.getHighFunction();
                List<ClangNode> nodes = new ArrayList<>();
                result.getCCodeMarkup().flatten(nodes);
                for (ClangNode node : nodes) {
                    monitor.checkCancelled();
                    String fact = null;
                    if (node instanceof ClangFieldToken) {
                        fact = structureFieldAccessFact(
                            function,
                            highFunction,
                            (ClangFieldToken) node,
                            null);
                    }
                    else if (node instanceof ClangBitFieldToken) {
                        fact = structureFieldAccessFact(
                            function,
                            highFunction,
                            null,
                            (ClangBitFieldToken) node);
                    }
                    if (fact == null) {
                        continue;
                    }
                    String key = fact.substring(fact.indexOf("\"access_id\": "));
                    if (seenAccesses.add(key)) {
                        structureFieldAccessFacts.add(fact);
                    }
                }
                decompiler.flushCache();
            }
        }
        finally {
            decompiler.dispose();
        }
    }

    private String structureFieldAccessFact(
            Function function,
            HighFunction highFunction,
            ClangFieldToken fieldToken,
            ClangBitFieldToken bitFieldToken) {
        DataType rawStructureType = fieldToken == null
            ? bitFieldToken.getDataType()
            : fieldToken.getDataType();
        Composite structureType = compositeDataType(rawStructureType);
        if (structureType == null) {
            return null;
        }

        DataTypeComponent component = fieldToken == null
            ? bitFieldToken.getComponent()
            : componentForOffset(structureType, fieldToken.getOffset());
        if (component == null) {
            return null;
        }

        PcodeOp op = fieldToken == null ? bitFieldToken.getPcodeOp() : fieldToken.getPcodeOp();
        String opcode = op == null ? "UNKNOWN" : op.getMnemonic();
        String entry = address(function.getEntryPoint());
        String accessAddress = pcodeAddress(op, fieldToken == null ? bitFieldToken : fieldToken);
        String pcodeOpId = pcodeOpId(op);
        DataType fieldType = component.getDataType();
        BitFieldDataType bitField = fieldType instanceof BitFieldDataType
            ? (BitFieldDataType) fieldType
            : null;
        DataType exportedFieldType = bitField == null ? fieldType : bitField.getBaseDataType();
        HighVariable rootVariable = highVariableForAccess(op);
        HighSymbol rootSymbol = rootVariable == null ? null : rootVariable.getSymbol();
        String fieldName = component.getFieldName();
        if (fieldName == null) {
            fieldName = component.getDefaultFieldName();
        }
        if (fieldName == null && fieldToken != null) {
            fieldName = fieldToken.getText();
        }
        String structureTypeId = dataTypeId(structureType);
        String fieldId = structureTypeId + ":field:" + component.getOrdinal() +
            ":0x" + Integer.toHexString(component.getOffset());
        String accessKind = structureFieldAccessKind(op, bitField != null);
        String accessId = "ghidra:structure_field_access:" + nullToToken(entry) + ":" +
            nullToToken(accessAddress) + ":" + nullToToken(opcode) + ":" +
            nullToToken(fieldId);

        return "    {" +
            "\"access_id\": " + json(accessId) + ", " +
            "\"function_id\": " + json(functionId(function)) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"address\": " + json(accessAddress) + ", " +
            "\"access_kind\": " + json(accessKind) + ", " +
            "\"structure_type_id\": " + json(structureTypeId) + ", " +
            "\"structure_name\": " + json(structureType.getName()) + ", " +
            "\"root_variable_id\": " + jsonOrNull(highVariableId(function, rootVariable, rootSymbol)) + ", " +
            "\"root_variable_name\": " + jsonOrNull(rootVariableName(rootVariable, rootSymbol)) + ", " +
            "\"root_storage\": " + jsonOrNull(rootVariableStorage(rootVariable, rootSymbol)) + ", " +
            "\"field_id\": " + json("ghidra:datatype_field:" + nullToToken(fieldId)) + ", " +
            "\"field_name\": " + jsonOrNull(fieldName) + ", " +
            "\"field_offset\": " + component.getOffset() + ", " +
            "\"field_byte_len\": " + nonNegativeIntegerOrNull(component.getLength()) + ", " +
            "\"field_data_type_id\": " + jsonOrNull(dataTypeId(exportedFieldType)) + ", " +
            "\"field_data_type_name\": " + jsonOrNull(displayName(fieldType)) + ", " +
            "\"field_data_type_kind\": " + jsonOrNull(dataTypeKind(exportedFieldType)) + ", " +
            "\"field_data_type_byte_len\": " + nonNegativeIntegerOrNull(
                exportedFieldType == null ? -1 : exportedFieldType.getLength()) + ", " +
            "\"pcode_opcode\": " + json(opcode) + ", " +
            "\"pcode_op_id\": " + jsonOrNull(pcodeOpId) + ", " +
            "\"statement_id\": null, " +
            "\"call_target_address\": " + jsonOrNull(callTargetAddress(op)) + ", " +
            "\"call_input_slot\": " + integerOrNull(callInputSlot(op)) + ", " +
            "\"pointer_relative_type\": " + jsonOrNull(pointerRelativeType(
                structureType,
                component,
                exportedFieldType)) + ", " +
            "\"recursive_call_depth\": 0, " +
            "\"creates_new_structure\": false, " +
            "\"extends_existing_structure\": true, " +
            "\"bit_offset\": " + integerOrNull(bitField == null ? null : bitField.getBitOffset()) + ", " +
            "\"bit_size\": " + integerOrNull(bitField == null ? null : bitField.getBitSize()) + ", " +
            "\"evidence_ids\": [" + json("ghidra:structure_field_access:" +
                nullToToken(accessAddress) + ":" + nullToToken(fieldId)) + "]" +
            "}";
    }

    private Composite compositeDataType(DataType dataType) {
        while (dataType instanceof TypeDef) {
            dataType = ((TypeDef) dataType).getDataType();
        }
        return dataType instanceof Composite ? (Composite) dataType : null;
    }

    private DataTypeComponent componentForOffset(Composite composite, int offset) {
        if (composite == null) {
            return null;
        }
        for (DataTypeComponent component : composite.getDefinedComponents()) {
            if (component.getOffset() == offset) {
                return component;
            }
        }
        for (DataTypeComponent component : composite.getDefinedComponents()) {
            int start = component.getOffset();
            int length = component.getLength();
            if (length > 0 && offset >= start && offset < start + length) {
                return component;
            }
        }
        return null;
    }

    private String pcodeAddress(PcodeOp op, ClangToken token) {
        if (op != null && op.getSeqnum() != null) {
            return address(op.getSeqnum().getTarget());
        }
        Address minAddress = token == null ? null : token.getMinAddress();
        return address(minAddress);
    }

    private String pcodeOpId(PcodeOp op) {
        if (op == null || op.getSeqnum() == null) {
            return null;
        }
        return "ghidra:pcode:" + address(op.getSeqnum().getTarget()) + ":" +
            op.getSeqnum().getTime();
    }

    private HighVariable highVariableForAccess(PcodeOp op) {
        if (op == null) {
            return null;
        }
        Varnode root = null;
        if (op.getOpcode() == PcodeOp.LOAD && op.getInputs().length > 1) {
            root = op.getInput(1);
        }
        else if (op.getOpcode() == PcodeOp.STORE && op.getInputs().length > 1) {
            root = op.getInput(1);
        }
        else if ((op.getOpcode() == PcodeOp.PTRADD || op.getOpcode() == PcodeOp.PTRSUB) &&
            op.getInputs().length > 0) {
            root = op.getInput(0);
        }
        else if (op.getOutput() != null) {
            root = op.getOutput();
        }
        if (root == null) {
            return null;
        }
        return root.getHigh();
    }

    private String highVariableId(
            Function function,
            HighVariable variable,
            HighSymbol symbol) {
        if (symbol != null) {
            return "ghidra:high_symbol:" + address(function.getEntryPoint()) + ":" +
                Long.toUnsignedString(symbol.getId());
        }
        if (variable == null) {
            return null;
        }
        return "ghidra:high_variable:" + address(function.getEntryPoint()) + ":" +
            nullToToken(variable.getName()) + ":" + variable.getSize();
    }

    private String rootVariableName(HighVariable variable, HighSymbol symbol) {
        if (symbol != null) {
            return symbol.getName();
        }
        return variable == null ? null : variable.getName();
    }

    private String rootVariableStorage(HighVariable variable, HighSymbol symbol) {
        if (symbol != null && symbol.getStorage() != null) {
            return symbol.getStorage().toString();
        }
        if (variable != null && variable.getRepresentative() != null) {
            return varnode(variable.getRepresentative());
        }
        return null;
    }

    private String structureFieldAccessKind(PcodeOp op, boolean bitField) {
        if (bitField) {
            return op != null && op.getOpcode() == PcodeOp.STORE
                ? "bit_field_write"
                : "bit_field_read";
        }
        if (op == null) {
            return "reference";
        }
        switch (op.getOpcode()) {
            case PcodeOp.LOAD:
                return "load";
            case PcodeOp.STORE:
                return "store";
            case PcodeOp.CALL:
            case PcodeOp.CALLIND:
                return "call_input";
            case PcodeOp.PTRADD:
                return "pointer_add";
            case PcodeOp.PTRSUB:
                return "pointer_sub";
            case PcodeOp.INT_ADD:
            case PcodeOp.INT_SUB:
                return "pointer_relative";
            default:
                return "reference";
        }
    }

    private String callTargetAddress(PcodeOp op) {
        if (op == null || (op.getOpcode() != PcodeOp.CALL && op.getOpcode() != PcodeOp.CALLIND) ||
            op.getInputs().length == 0) {
            return null;
        }
        Varnode target = op.getInput(0);
        return target == null ? null : address(target.getAddress());
    }

    private Integer callInputSlot(PcodeOp op) {
        if (op == null || (op.getOpcode() != PcodeOp.CALL && op.getOpcode() != PcodeOp.CALLIND)) {
            return null;
        }
        return op.getInputs().length <= 1 ? null : Integer.valueOf(1);
    }

    private String pointerRelativeType(
            Composite structureType,
            DataTypeComponent component,
            DataType fieldType) {
        if (structureType == null || component == null) {
            return null;
        }
        return nullToToken(structureType.getName()) + "_offset_" + component.getOffset() + "_" +
            nullToToken(displayName(fieldType));
    }

    private String jumpTableFact(Function function, JumpTable table) {
        Address switchAddressValue = table.getSwitchAddress();
        if (switchAddressValue == null) {
            return null;
        }
        Address[] cases;
        Integer[] labelValues;
        JumpTable.LoadTable[] loadTables;
        try {
            cases = table.getCases();
            labelValues = table.getLabelValues();
            loadTables = table.getLoadTables();
        }
        catch (RuntimeException e) {
            return null;
        }
        if (cases == null || cases.length == 0) {
            return null;
        }

        String entry = address(function.getEntryPoint());
        String switchAddress = address(switchAddressValue);
        String jumpTableId = "ghidra:jumptable:" + entry + ":" + switchAddress;
        List<String> caseFacts = new ArrayList<>();
        for (int i = 0; i < cases.length; i++) {
            Address destination = cases[i];
            if (destination == null) {
                continue;
            }
            Integer labelValue = i < labelValues.length ? labelValues[i] : null;
            boolean isDefault = isDefaultCase(labelValues, i);
            String label = isDefault
                ? "default"
                : (labelValue == null ? "case_" + i : "case_" + labelValue);
            caseFacts.add("      {" +
                "\"case_id\": " + json(jumpTableId + ":case:" + i) + ", " +
                "\"destination\": " + json(address(destination)) + ", " +
                "\"label_value\": " + integerOrNull(labelValue) + ", " +
                "\"is_default\": " + isDefault + ", " +
                "\"label\": " + json(label) + ", " +
                "\"evidence_ids\": [" + json("ghidra:jumptable_case:" + switchAddress + ":" + i) + "]" +
                "}");
        }
        if (caseFacts.isEmpty()) {
            return null;
        }

        List<String> loadTableFacts = new ArrayList<>();
        for (int i = 0; i < loadTables.length; i++) {
            JumpTable.LoadTable loadTable = loadTables[i];
            if (loadTable == null || loadTable.getAddress() == null ||
                loadTable.getSize() <= 0 || loadTable.getNum() <= 0) {
                continue;
            }
            loadTableFacts.add("      {" +
                "\"load_table_id\": " + json(jumpTableId + ":load:" + i) + ", " +
                "\"address\": " + json(address(loadTable.getAddress())) + ", " +
                "\"entry_byte_len\": " + loadTable.getSize() + ", " +
                "\"entry_count\": " + loadTable.getNum() + ", " +
                "\"interpreted_as_pointer_table\": " + isPointerLoadTable(loadTable, cases) + ", " +
                "\"evidence_ids\": [" + json("ghidra:jumptable_load:" + switchAddress + ":" + i) + "]" +
                "}");
        }

        String displayFormat = displayFormat(function, switchAddressValue);
        return "    {" +
            "\"jump_table_id\": " + json(jumpTableId) + ", " +
            "\"function_id\": " + json("ghidra:function:" + entry) + ", " +
            "\"entry_point\": " + json(entry) + ", " +
            "\"switch_address\": " + json(switchAddress) + ", " +
            "\"switch_statement_id\": null, " +
            "\"display_format\": " + jsonOrNull(displayFormat) + ", " +
            "\"cases\": [\n" + String.join(",\n", caseFacts) + "\n    ], " +
            "\"load_tables\": [\n" + String.join(",\n", loadTableFacts) + "\n    ], " +
            "\"override_applied\": false, " +
            "\"references_complete\": true, " +
            "\"evidence_ids\": [" + json("ghidra:jumptable:" + switchAddress) + "]" +
            "}";
    }

    private boolean isDefaultCase(Integer[] caseValues, int caseIndex) {
        return caseIndex == caseValues.length ||
            (caseIndex < caseValues.length && caseValues[caseIndex] != null &&
                caseValues[caseIndex] == DEFAULT_CASE_VALUE);
    }

    private boolean isPointerLoadTable(JumpTable.LoadTable loadTable, Address[] switchCases) {
        if (switchCases.length == 0 || loadTable.getSize() > 8) {
            return false;
        }
        int size = loadTable.getSize();
        int defaultPointerSize = currentProgram.getDefaultPointerSize();
        if (size != defaultPointerSize ||
            size != switchCases[0].getAddressSpace().getPointerSize()) {
            return false;
        }
        try {
            int addressableUnitSize = switchCases[0].getAddressSpace().getAddressableUnitSize();
            boolean bigEndian = currentProgram.getLanguage().isBigEndian();
            for (int i = 0; i < loadTable.getNum(); i++) {
                Address entryAddress = loadTable.getAddress().add((long) size * i);
                byte[] raw = new byte[size];
                currentProgram.getMemory().getBytes(entryAddress, raw);
                long unsignedOffset = bytesToUnsigned(raw, bigEndian) * addressableUnitSize;
                long signedOffset = signExtend(unsignedOffset / addressableUnitSize, size) *
                    addressableUnitSize;
                boolean found = false;
                for (Address caseAddress : switchCases) {
                    long offset = caseAddress.getOffset();
                    if (offset == unsignedOffset || offset == signedOffset) {
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    return false;
                }
            }
            return true;
        }
        catch (Exception e) {
            return false;
        }
    }

    private long bytesToUnsigned(byte[] raw, boolean bigEndian) {
        long value = 0;
        if (bigEndian) {
            for (byte b : raw) {
                value = (value << 8) | (b & 0xffL);
            }
        }
        else {
            for (int i = raw.length - 1; i >= 0; i--) {
                value = (value << 8) | (raw[i] & 0xffL);
            }
        }
        return value;
    }

    private long signExtend(long value, int byteLen) {
        int shift = (8 - byteLen) * 8;
        return (value << shift) >> shift;
    }

    private String displayFormat(Function function, Address switchAddress) {
        int format = JumpTable.getFormatOverride(function, switchAddress);
        if (format == EquateSymbol.FORMAT_DEFAULT) {
            return null;
        }
        return EquateSymbol.getIntegerFormatString(format);
    }

    private int intArg(String[] args, int index, int fallback) {
        if (args.length <= index) {
            return fallback;
        }
        try {
            return Integer.parseInt(args[index]);
        }
        catch (NumberFormatException e) {
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

    private String varnode(Varnode value) {
        if (value == null) {
            return null;
        }
        return address(value.getAddress()) + ":" + value.getSize();
    }

    private String varnodeArray(Varnode[] values) {
        if (values == null || values.length == 0) {
            return "[]";
        }
        List<String> encoded = new ArrayList<>();
        for (Varnode value : values) {
            encoded.add(json(varnode(value)));
        }
        return "[" + String.join(", ", encoded) + "]";
    }

    private String semanticRoles(Reference reference) {
        RefType type = reference.getReferenceType();
        List<String> roles = new ArrayList<>();
        if (reference.isPrimary()) {
            roles.add("primary");
        }
        if (reference.isExternalReference()) {
            roles.add("external");
        }
        if (reference.isMemoryReference()) {
            roles.add("memory");
        }
        if (reference.isRegisterReference()) {
            roles.add("register");
        }
        if (reference.isStackReference()) {
            roles.add("stack");
        }
        if (reference.isOperandReference()) {
            roles.add("operand");
        }
        if (reference.isMnemonicReference()) {
            roles.add("mnemonic");
        }
        if (reference.isEntryPointReference()) {
            roles.add("entry_point");
        }
        if (reference.isOffsetReference()) {
            roles.add("offset");
        }
        if (reference.isShiftedReference()) {
            roles.add("shifted");
        }
        if (type.isData()) {
            roles.add("data");
        }
        if (type.isRead()) {
            roles.add("read");
        }
        if (type.isWrite()) {
            roles.add("write");
        }
        if (type.isFlow()) {
            roles.add("flow");
        }
        if (type.isCall()) {
            roles.add("call");
        }
        if (type.isJump()) {
            roles.add("jump");
        }
        if (type.isConditional()) {
            roles.add("conditional");
        }
        if (type.isComputed()) {
            roles.add("computed");
        }
        if (type.isTerminal()) {
            roles.add("terminal");
        }
        if (type.isOverride()) {
            roles.add("override");
        }
        return stringArray(roles);
    }

    private String stringArray(List<String> values) {
        if (values.isEmpty()) {
            return "[]";
        }
        List<String> encoded = new ArrayList<>();
        for (String value : values) {
            encoded.add(json(value));
        }
        return "[" + String.join(", ", encoded) + "]";
    }

    private List<String> uniqueStringsWithout(List<String> values, String excluded) {
        List<String> unique = new ArrayList<>();
        Set<String> seen = new HashSet<>();
        for (String value : values) {
            if (value == null || value.trim().isEmpty()) {
                continue;
            }
            String normalized = value.trim();
            if (normalized.equals(excluded) || !seen.add(normalized)) {
                continue;
            }
            unique.add(normalized);
        }
        return unique;
    }

    private String nestedObjectArray(List<String> values, String indent) {
        if (values.isEmpty()) {
            return "[]";
        }
        return "[\n" + String.join(",\n", values) + "\n" + indent + "]";
    }

    private String nullToToken(String value) {
        if (value == null || value.trim().isEmpty()) {
            return "null";
        }
        return value.trim().replaceAll("[^A-Za-z0-9_.:-]+", "_");
    }

    private String jsonOrNull(String value) {
        return value == null ? "null" : json(value);
    }

    private String integerOrNull(Integer value) {
        return value == null ? "null" : value.toString();
    }

    private String longOrNull(Long value) {
        return value == null || value < 0 ? "null" : value.toString();
    }

    private String booleanOrNull(Boolean value) {
        return value == null ? "null" : value.toString();
    }

    private String nonNegativeIntegerOrNull(int value) {
        return value < 0 ? "null" : Integer.toString(value);
    }

    private int nonNegative(int value) {
        return value < 0 ? 0 : value;
    }

    private String shortOrNull(short value) {
        return value < 0 ? "null" : Short.toString(value);
    }

    private String unsignedLongOrNull(long value) {
        return value == 0 ? "null" : Long.toUnsignedString(value);
    }

    private String equateFormat(String name, String displayValue) {
        String rendered = displayValue == null ? name : displayValue;
        if (rendered == null) {
            return null;
        }
        String value = rendered.trim().toLowerCase();
        if (value.startsWith("0x") || value.endsWith("h")) {
            return "hex";
        }
        if (value.startsWith("0b")) {
            return "bin";
        }
        if (value.startsWith("0o")) {
            return "oct";
        }
        if (value.startsWith("'") && value.endsWith("'")) {
            return "char";
        }
        return null;
    }

    private String emptyToNull(String value) {
        return value == null || value.trim().isEmpty() ? null : value;
    }

    private String firstNonEmpty(String... values) {
        for (String value : values) {
            if (value != null && !value.trim().isEmpty()) {
                return value.trim();
            }
        }
        return "unknown";
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
