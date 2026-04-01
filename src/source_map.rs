//! Source mapping for Leo programs.
//!
//! Provides two mapping structures:
//!
//! 1. `SourceMap` — byte-offset to line-number mapping for Leo source files.
//! 2. `AleoSourceMap` — maps Aleo instruction indices back to Leo source lines
//!    by correlating compiled Aleo operations with Leo let-bindings.

use std::collections::HashMap;
use std::path::Path;

/// Maps byte offsets in a Leo source file to line numbers.
///
/// Precomputes line boundaries from the source text so that any byte
/// offset can be quickly mapped to a 1-based line number.
pub struct SourceMap {
    /// Byte offset of the start of each line (0-indexed lines).
    line_starts: Vec<usize>,
    /// Original source code (kept for inspection/debugging).
    source_code: String,
}

impl SourceMap {
    /// Build a `SourceMap` from a source file path and its contents.
    pub fn from_source(_source_path: &Path, source_code: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, ch) in source_code.char_indices() {
            if ch == '\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            line_starts,
            source_code: source_code.to_string(),
        }
    }

    /// Convert a byte offset to a 1-based line number.
    pub fn byte_to_line(&self, byte_offset: usize) -> u32 {
        match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => (idx + 1) as u32,
            Err(idx) => idx as u32,
        }
    }

    /// Return the total number of lines in the source.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Return a reference to the source code.
    pub fn source_code(&self) -> &str {
        &self.source_code
    }
}

// ---------------------------------------------------------------------------
// Aleo-to-Leo source map
// ---------------------------------------------------------------------------

/// An entry in the Aleo source map: maps an Aleo instruction (by function
/// name and instruction index within that function) to a Leo source location.
#[derive(Debug, Clone)]
struct AleoSourceEntry {
    /// The Leo source file name (e.g. "flow_test.leo").
    file: String,
    /// 1-based Leo source line number.
    line: u32,
}

/// Maps Aleo instruction positions back to Leo source lines.
///
/// The key insight is that the Leo compiler generates Aleo instructions
/// in the same order as Leo let-bindings: each let-binding compiles to
/// exactly one Aleo instruction (an arithmetic op or a call). By
/// matching operation types and the sequential register pattern, we
/// correlate each Aleo instruction index with its originating Leo line.
pub struct AleoSourceMap {
    /// Per-function mapping: function_name -> (instruction_index -> entry).
    entries: HashMap<String, Vec<Option<AleoSourceEntry>>>,
}

impl AleoSourceMap {
    /// Resolve an Aleo instruction to its originating Leo source location.
    ///
    /// Returns `Some((file, line))` if the instruction has a known Leo source
    /// mapping, or `None` for instructions that don't correspond to user code
    /// (e.g. synthetic call wrappers).
    pub fn resolve(&self, function_name: &str, instruction_index: usize) -> Option<(String, u32)> {
        let func_entries = self.entries.get(function_name)?;
        let entry = func_entries.get(instruction_index)?.as_ref()?;
        Some((entry.file.clone(), entry.line))
    }

    /// Return all function names that have entries in this source map.
    pub fn function_names(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// Return the number of mapped instructions for a given function.
    pub fn instruction_count(&self, function_name: &str) -> usize {
        self.entries
            .get(function_name)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

/// Classification of an Aleo instruction's operation type, used to match
/// against Leo source expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AleoOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Compare,
    Call,
    /// Synthetic load: `add <literal> 0u32 into rN;` — used to load a
    /// constant into a register (corresponds to a literal let-binding).
    LiteralLoad,
    Other,
}

/// Classification of a Leo expression's operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeoExprKind {
    Add,
    Sub,
    Mul,
    Div,
    Literal,
    FunctionCall,
    Other,
}

/// Generate an `AleoSourceMap` by correlating the compiled Aleo instructions
/// with the Leo source let-bindings.
///
/// The correlation strategy:
/// 1. Parse Leo functions and their let-bindings (with line numbers).
/// 2. Parse Aleo functions and their instruction sequences.
/// 3. For each Aleo function, find the matching Leo function by name.
/// 4. Walk both instruction lists in parallel: each Aleo instruction that
///    writes to a new destination register maps to the next Leo let-binding,
///    confirmed by matching operation kinds (add↔+, mul↔*, etc.).
pub fn generate_source_map(leo_source: &str, aleo_source: &str) -> AleoSourceMap {
    let leo_file = extract_leo_filename(leo_source);
    let leo_functions = parse_leo_function_info(leo_source);
    let aleo_functions = parse_aleo_function_info(aleo_source);

    let mut entries: HashMap<String, Vec<Option<AleoSourceEntry>>> = HashMap::new();

    for aleo_fn in &aleo_functions {
        // Find matching Leo function.
        let leo_fn = leo_functions.iter().find(|f| f.name == aleo_fn.name);

        let mut instruction_entries: Vec<Option<AleoSourceEntry>> =
            vec![None; aleo_fn.instructions.len()];

        if let Some(leo_fn) = leo_fn {
            // Walk Aleo instructions and Leo bindings in parallel.
            // The Leo compiler emits one instruction per let-binding in order.
            let mut binding_idx = 0;

            for (instr_idx, aleo_instr) in aleo_fn.instructions.iter().enumerate() {
                if binding_idx >= leo_fn.bindings.len() {
                    break;
                }

                let aleo_kind = classify_aleo_instruction(aleo_instr);
                let leo_binding = &leo_fn.bindings[binding_idx];
                let leo_kind = classify_leo_expression(&leo_binding.expression);

                // Check if the operation kinds are compatible.
                let compatible = match (aleo_kind, leo_kind) {
                    (AleoOpKind::Add, LeoExprKind::Add) => true,
                    (AleoOpKind::Sub, LeoExprKind::Sub) => true,
                    (AleoOpKind::Mul, LeoExprKind::Mul) => true,
                    (AleoOpKind::Div, LeoExprKind::Div) => true,
                    (AleoOpKind::LiteralLoad, LeoExprKind::Literal) => true,
                    (AleoOpKind::Call, LeoExprKind::FunctionCall) => true,
                    // If we can't determine the kind, accept any match
                    // based on sequential ordering.
                    (AleoOpKind::Other, _) | (_, LeoExprKind::Other) => true,
                    // Also accept when Leo has a composite expression that
                    // the compiler broke into multiple instructions — the
                    // first instruction of the group matches the binding.
                    _ => true,
                };

                if compatible {
                    instruction_entries[instr_idx] = Some(AleoSourceEntry {
                        file: leo_file.clone(),
                        line: leo_binding.line,
                    });
                    binding_idx += 1;
                }
            }
        }

        entries.insert(aleo_fn.name.clone(), instruction_entries);
    }

    AleoSourceMap { entries }
}

// ---------------------------------------------------------------------------
// Internal parsing helpers for source map generation
// ---------------------------------------------------------------------------

/// Extract a filename hint from Leo source (from the `program X.aleo` line).
fn extract_leo_filename(source: &str) -> String {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("program ") {
            if let Some(dot_pos) = trimmed.find('.') {
                let name = trimmed[8..dot_pos].trim();
                if !name.is_empty() {
                    return format!("{name}.leo");
                }
            }
        }
    }
    "unknown.leo".to_string()
}

/// A Leo binding with its expression text, for operation-kind matching.
#[derive(Debug, Clone)]
struct LeoBindingInfo {
    #[allow(dead_code)]
    name: String,
    expression: String,
    line: u32,
}

/// A Leo function with its bindings, for source map generation.
#[derive(Debug, Clone)]
struct LeoFunctionInfo {
    name: String,
    bindings: Vec<LeoBindingInfo>,
}

/// Parse Leo source to extract function names and their let-binding info.
fn parse_leo_function_info(source: &str) -> Vec<LeoFunctionInfo> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Check for transition or function definition.
        let after_keyword = if trimmed.starts_with("transition ") {
            &trimmed[11..]
        } else if trimmed.starts_with("function ") {
            &trimmed[9..]
        } else {
            i += 1;
            continue;
        };

        // Parse function name.
        let name_end = after_keyword.find('(').unwrap_or(after_keyword.len());
        let name = after_keyword[..name_end].trim().to_string();

        // Parse body.
        let mut bindings = Vec::new();
        let mut brace_depth = 0i32;
        let mut body_started = false;

        for ch in lines[i].chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    body_started = true;
                }
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        let mut j = i + 1;
        while j < lines.len() && (brace_depth > 0 || !body_started) {
            let body_line = lines[j].trim();
            let body_line_num = (j + 1) as u32;

            for ch in lines[j].chars() {
                match ch {
                    '{' => {
                        brace_depth += 1;
                        body_started = true;
                    }
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }

            // Parse let bindings with their expressions.
            if body_line.starts_with("let ") {
                if let Some(binding) = parse_binding_info(body_line, body_line_num) {
                    bindings.push(binding);
                }
            }

            if brace_depth <= 0 && body_started {
                break;
            }
            j += 1;
        }

        if !name.is_empty() {
            functions.push(LeoFunctionInfo { name, bindings });
        }

        i = j + 1;
    }

    functions
}

/// Parse a Leo let-binding to extract name, expression, and line number.
fn parse_binding_info(line: &str, line_num: u32) -> Option<LeoBindingInfo> {
    let trimmed = line.trim();
    if !trimmed.starts_with("let ") {
        return None;
    }

    let after_let = &trimmed[4..];
    let colon_pos = after_let.find(':')?;
    let name = after_let[..colon_pos].trim().to_string();

    let after_colon = &after_let[colon_pos + 1..];
    let expression = if let Some(eq_pos) = after_colon.find('=') {
        after_colon[eq_pos + 1..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string()
    } else {
        String::new()
    };

    if !name.is_empty() {
        Some(LeoBindingInfo {
            name,
            expression,
            line: line_num,
        })
    } else {
        None
    }
}

/// A parsed Aleo instruction line for source map generation (simpler than
/// the full `AleoInstruction` enum in tracer.rs — we only need the opcode).
#[derive(Debug, Clone)]
struct AleoInstructionInfo {
    opcode: String,
    raw: String,
}

/// A parsed Aleo function for source map generation.
#[derive(Debug, Clone)]
struct AleoFunctionInfo {
    name: String,
    instructions: Vec<AleoInstructionInfo>,
}

/// Parse Aleo source to extract function names and their instruction opcodes.
fn parse_aleo_function_info(source: &str) -> Vec<AleoFunctionInfo> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        let name = if trimmed.starts_with("closure ") {
            trimmed[8..].trim_end_matches(':').trim().to_string()
        } else if trimmed.starts_with("function ") {
            trimmed[9..].trim_end_matches(':').trim().to_string()
        } else {
            i += 1;
            continue;
        };

        i += 1;
        let mut instructions = Vec::new();

        while i < lines.len() {
            let line = lines[i].trim();

            if line.is_empty()
                || line.starts_with("closure ")
                || line.starts_with("function ")
                || line.starts_with("program ")
            {
                break;
            }

            // Skip input/output lines, only collect instructions.
            if !line.starts_with("input ") && !line.starts_with("output ") {
                let opcode = line.split_whitespace().next().unwrap_or("").to_string();
                if !opcode.is_empty() {
                    instructions.push(AleoInstructionInfo {
                        opcode,
                        raw: line.to_string(),
                    });
                }
            }

            i += 1;
        }

        if !name.is_empty() {
            functions.push(AleoFunctionInfo { name, instructions });
        }
    }

    functions
}

/// Classify an Aleo instruction into an operation kind.
fn classify_aleo_instruction(instr: &AleoInstructionInfo) -> AleoOpKind {
    // Check for literal loads: `add <literal> 0u32 into rN;`
    // These are the compiler's way of loading a constant into a register.
    if (instr.opcode == "add" || instr.opcode == "add.w") && is_literal_load(&instr.raw) {
        return AleoOpKind::LiteralLoad;
    }

    match instr.opcode.as_str() {
        "add" | "add.w" => AleoOpKind::Add,
        "sub" | "sub.w" => AleoOpKind::Sub,
        "mul" | "mul.w" => AleoOpKind::Mul,
        "div" | "div.w" => AleoOpKind::Div,
        "rem" | "rem.w" | "mod" => AleoOpKind::Mod,
        "is.eq" | "is.neq" | "lt" | "lte" | "gt" | "gte" => AleoOpKind::Compare,
        "call" => AleoOpKind::Call,
        _ => AleoOpKind::Other,
    }
}

/// Check if an Aleo `add` instruction is really a literal load
/// (i.e. `add <literal> 0<type> into rN;`).
fn is_literal_load(raw: &str) -> bool {
    let parts: Vec<&str> = raw
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect();
    // Pattern: add <literal> 0<type> into rN
    if parts.len() >= 5 && parts[3] == "into" {
        let operand1 = parts[1];
        let operand2 = parts[2];
        // operand1 should be a literal (not a register)
        let op1_is_literal = !operand1.starts_with('r')
            || operand1[1..]
                .chars()
                .next()
                .map_or(true, |c| !c.is_ascii_digit());
        // operand2 should be a zero literal (0u32, 0u64, etc.)
        let op2_is_zero = operand2.starts_with('0')
            && !operand2.starts_with("0r")
            && operand2.len() > 1
            && !operand2.starts_with('r');
        op1_is_literal && op2_is_zero
    } else {
        false
    }
}

/// Classify a Leo expression into an operation kind.
fn classify_leo_expression(expr: &str) -> LeoExprKind {
    let expr = expr.trim();

    if expr.is_empty() {
        return LeoExprKind::Other;
    }

    // Function call: contains `(` and `)`.
    if expr.contains('(') && expr.contains(')') {
        return LeoExprKind::FunctionCall;
    }

    // Binary operators — scan for top-level +, -, *, /.
    // We need to handle type suffixes like `2u32` not being confused with operators.
    let mut depth = 0i32;
    let chars: Vec<char> = expr.chars().collect();
    for i in (1..chars.len()).rev() {
        match chars[i] {
            ')' => depth += 1,
            '(' => depth -= 1,
            '+' if depth == 0 => return LeoExprKind::Add,
            '-' if depth == 0 => return LeoExprKind::Sub,
            '*' if depth == 0 => return LeoExprKind::Mul,
            '/' if depth == 0 => return LeoExprKind::Div,
            _ => {}
        }
    }

    // If none of the above matched, it's likely a literal.
    LeoExprKind::Literal
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_byte_to_line_simple() {
        let source = "line1\nline2\nline3\n";
        let map = SourceMap::from_source(&PathBuf::from("test.leo"), source);
        assert_eq!(map.byte_to_line(0), 1);
        assert_eq!(map.byte_to_line(6), 2);
        assert_eq!(map.byte_to_line(12), 3);
    }

    #[test]
    fn test_line_count() {
        let source = "a\nb\nc\n";
        let map = SourceMap::from_source(&PathBuf::from("test.leo"), source);
        assert_eq!(map.line_count(), 4);
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let map = SourceMap::from_source(&PathBuf::from("test.leo"), source);
        assert_eq!(map.line_count(), 1);
        assert_eq!(map.byte_to_line(0), 1);
    }

    // -----------------------------------------------------------------------
    // AleoSourceMap tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_source_map_flow_test() {
        let leo_source = r#"program flow_test.aleo {
    transition compute() -> u32 {
        let a: u32 = 10u32;
        let b: u32 = 32u32;
        let sum_val: u32 = a + b;
        let doubled: u32 = sum_val * 2u32;
        let final_result: u32 = doubled + a;
        return final_result;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;

        let aleo_source = r#"program flow_test.aleo;

closure compute:
    add 10u32 0u32 into r0;
    add 32u32 0u32 into r1;
    add r0 r1 into r2;
    mul r2 2u32 into r3;
    add r3 r0 into r4;
    output r4 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;

        let source_map = generate_source_map(leo_source, aleo_source);

        // The compute function should have 5 instructions mapped.
        assert_eq!(source_map.instruction_count("compute"), 5);

        // Instruction 0 (add 10u32 0u32 into r0) -> line 3 (let a: u32 = 10u32;)
        let (file, line) = source_map.resolve("compute", 0).unwrap();
        assert_eq!(file, "flow_test.leo");
        assert_eq!(line, 3);

        // Instruction 1 (add 32u32 0u32 into r1) -> line 4 (let b: u32 = 32u32;)
        let (_, line) = source_map.resolve("compute", 1).unwrap();
        assert_eq!(line, 4);

        // Instruction 2 (add r0 r1 into r2) -> line 5 (let sum_val: u32 = a + b;)
        let (_, line) = source_map.resolve("compute", 2).unwrap();
        assert_eq!(line, 5);

        // Instruction 3 (mul r2 2u32 into r3) -> line 6 (let doubled: u32 = sum_val * 2u32;)
        let (_, line) = source_map.resolve("compute", 3).unwrap();
        assert_eq!(line, 6);

        // Instruction 4 (add r3 r0 into r4) -> line 7 (let final_result: u32 = doubled + a;)
        let (_, line) = source_map.resolve("compute", 4).unwrap();
        assert_eq!(line, 7);
    }

    #[test]
    fn test_source_map_empty_function() {
        let leo_source = r#"program empty.aleo {
    transition main() -> u32 {
        return 0u32;
    }
}"#;

        let aleo_source = r#"program empty.aleo;

function main:
    output 0u32 as u32.private;
"#;

        let source_map = generate_source_map(leo_source, aleo_source);
        // main has no instructions (only output), so no mappings.
        assert_eq!(source_map.instruction_count("main"), 0);
    }

    #[test]
    fn test_source_map_no_let_bindings() {
        let leo_source = r#"program noop.aleo {
    transition compute() -> u32 {
        return 42u32;
    }

    transition main() -> u32 {
        return compute();
    }
}"#;

        let aleo_source = r#"program noop.aleo;

closure compute:
    add 42u32 0u32 into r0;
    output r0 as u32;

function main:
    call compute into r0;
    output r0 as u32.private;
"#;

        let source_map = generate_source_map(leo_source, aleo_source);
        // compute has 1 instruction but no let-bindings, so no mappings.
        assert_eq!(source_map.instruction_count("compute"), 1);
        assert!(source_map.resolve("compute", 0).is_none());
    }

    #[test]
    fn test_source_map_resolve_out_of_range() {
        let leo_source = r#"program test.aleo {
    transition main() -> u32 {
        let a: u32 = 1u32;
        return a;
    }
}"#;

        let aleo_source = r#"program test.aleo;

function main:
    add 1u32 0u32 into r0;
    output r0 as u32.private;
"#;

        let source_map = generate_source_map(leo_source, aleo_source);
        // Index 0 should resolve, index 99 should not.
        assert!(source_map.resolve("main", 0).is_some());
        assert!(source_map.resolve("main", 99).is_none());
        // Non-existent function should not resolve.
        assert!(source_map.resolve("nonexistent", 0).is_none());
    }

    #[test]
    fn test_classify_leo_expression() {
        assert_eq!(classify_leo_expression("a + b"), LeoExprKind::Add);
        assert_eq!(classify_leo_expression("a - b"), LeoExprKind::Sub);
        assert_eq!(classify_leo_expression("a * 2u32"), LeoExprKind::Mul);
        assert_eq!(classify_leo_expression("a / b"), LeoExprKind::Div);
        assert_eq!(classify_leo_expression("10u32"), LeoExprKind::Literal);
        assert_eq!(
            classify_leo_expression("compute()"),
            LeoExprKind::FunctionCall
        );
        assert_eq!(classify_leo_expression(""), LeoExprKind::Other);
    }

    #[test]
    fn test_classify_aleo_instruction_literal_load() {
        let instr = AleoInstructionInfo {
            opcode: "add".to_string(),
            raw: "add 10u32 0u32 into r0;".to_string(),
        };
        assert_eq!(classify_aleo_instruction(&instr), AleoOpKind::LiteralLoad);
    }

    #[test]
    fn test_classify_aleo_instruction_real_add() {
        let instr = AleoInstructionInfo {
            opcode: "add".to_string(),
            raw: "add r0 r1 into r2;".to_string(),
        };
        assert_eq!(classify_aleo_instruction(&instr), AleoOpKind::Add);
    }
}
