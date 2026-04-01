//! Transition function tracing for Leo programs (M6 milestone).
//!
//! Handles Leo `transition` functions and `record` types, which are the
//! primary constructs for off-chain ZK execution. Records are private
//! data containers with an owner address, gates (fee credits), and
//! custom fields. Transitions are the main entry points that consume
//! and produce records.
//!
//! Built-in variables `self.caller` and `self.signer` are resolved
//! through a `TransitionContext` that simulates the execution environment.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

/// A Leo record value with owner, gates, and custom fields.
///
/// In Leo, a `record` is declared like:
/// ```leo
/// record token {
///     owner: address,
///     gates: u64,
///     amount: u64,
/// }
/// ```
///
/// Records are private data — each has an `owner` (address), `gates`
/// (u64 fee credits), and zero or more custom fields.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordType {
    /// The record type name (e.g. "token").
    pub name: String,
    /// The owner address.
    pub owner: String,
    /// The gates value (u64 fee credits, stored as i64).
    pub gates: i64,
    /// Custom fields: (field_name, field_value).
    /// Values are stored as i64 to match the AVM interpreter's register type.
    pub fields: Vec<(String, i64)>,
}

/// A parsed record type definition from Leo source (schema, not instance).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDefinition {
    /// The record type name.
    pub name: String,
    /// Field names and their type names, in declaration order.
    /// Includes `owner` and `gates` if declared explicitly.
    pub fields: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Transition context
// ---------------------------------------------------------------------------

/// Execution context for a transition function.
///
/// Provides the built-in `self.caller` and `self.signer` values that
/// are available inside transition functions.
#[derive(Debug, Clone)]
pub struct TransitionContext {
    /// The address of the caller (the account that invoked the transition).
    pub caller: String,
    /// The address of the signer (the account that signed the transaction).
    pub signer: String,
}

impl TransitionContext {
    /// Create a new transition context with the given caller and signer.
    pub fn new(caller: &str, signer: &str) -> Self {
        Self {
            caller: caller.to_string(),
            signer: signer.to_string(),
        }
    }

    /// Create a default context with a test address for both caller and signer.
    pub fn default_test() -> Self {
        Self {
            caller: "aleo1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq3ljyzc"
                .to_string(),
            signer: "aleo1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq3ljyzc"
                .to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Transition trace events
// ---------------------------------------------------------------------------

/// Trace events specific to transition function execution.
#[derive(Debug, Clone)]
pub enum TransitionTraceEvent {
    /// A transition function was entered.
    TransitionEntry {
        /// The transition function name.
        function_name: String,
        /// Whether the function is a transition (true) or a regular function (false).
        is_transition: bool,
    },
    /// A record field was accessed.
    RecordFieldAccess {
        /// The record variable name.
        record_name: String,
        /// The field being accessed.
        field_name: String,
        /// The resolved value (as a string for addresses, numeric for others).
        value: String,
    },
    /// A built-in variable was resolved (self.caller, self.signer).
    BuiltinResolved {
        /// The built-in name (e.g. "self.caller").
        name: String,
        /// The resolved value.
        value: String,
    },
    /// A record was produced as output.
    RecordOutput {
        /// The record type name.
        record_type: String,
        /// The owner address.
        owner: String,
        /// The gates value.
        gates: i64,
        /// Custom field values.
        fields: Vec<(String, i64)>,
    },
}

// ---------------------------------------------------------------------------
// Record parsing
// ---------------------------------------------------------------------------

/// Parse record type definitions from Leo source code.
///
/// Finds all `record <name> { ... }` blocks and extracts their field
/// definitions. Returns a list of `RecordDefinition` values.
///
/// Example input:
/// ```text
/// record token {
///     owner: address,
///     gates: u64,
///     amount: u64,
/// }
/// ```
pub fn parse_record_types(source: &str) -> Vec<RecordDefinition> {
    let mut records = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Look for `record <name> {`
        if trimmed.starts_with("record ") {
            let after = trimmed.strip_prefix("record ").unwrap();
            // Extract name (before '{' or whitespace).
            let name_end = after
                .find(|c: char| c == '{' || c.is_whitespace())
                .unwrap_or(after.len());
            let name = after[..name_end].trim().to_string();

            if name.is_empty() {
                i += 1;
                continue;
            }

            // Check if the opening brace is on this line.
            let has_open_brace = trimmed.contains('{');
            let mut fields = Vec::new();
            let mut brace_depth = if has_open_brace { 1i32 } else { 0 };

            let mut j = i + 1;
            while j < lines.len() {
                let field_line = lines[j].trim();

                // Track braces.
                for ch in field_line.chars() {
                    match ch {
                        '{' => brace_depth += 1,
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                }

                if brace_depth <= 0 {
                    break;
                }

                // Parse field: `name: type,` or `name: type`
                let field_trimmed = field_line
                    .trim()
                    .trim_end_matches(',')
                    .trim_end_matches(';');
                if let Some(colon_pos) = field_trimmed.find(':') {
                    let field_name = field_trimmed[..colon_pos].trim().to_string();
                    let field_type = field_trimmed[colon_pos + 1..].trim().to_string();
                    if !field_name.is_empty()
                        && !field_type.is_empty()
                        && !field_name.starts_with("//")
                        && !field_name.starts_with('}')
                    {
                        fields.push((field_name, field_type));
                    }
                }

                j += 1;
            }

            records.push(RecordDefinition { name, fields });

            i = j + 1;
        } else {
            i += 1;
        }
    }

    records
}

/// Resolve a built-in variable name to its value.
///
/// Supported built-ins:
/// - `self.caller` -> the caller address
/// - `self.signer` -> the signer address
pub fn resolve_builtin(name: &str, ctx: &TransitionContext) -> Option<String> {
    match name {
        "self.caller" => Some(ctx.caller.clone()),
        "self.signer" => Some(ctx.signer.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Transition detection
// ---------------------------------------------------------------------------

/// Information about a parsed Leo function/transition declaration.
#[derive(Debug, Clone)]
pub struct TransitionInfo {
    /// The function name.
    pub name: String,
    /// Whether this is declared as `transition` (true) or `function` (false).
    pub is_transition: bool,
    /// 1-based line number of the declaration.
    pub line: u32,
    /// Parameter names and types (includes record-typed params).
    pub params: Vec<(String, String)>,
    /// Return type (if any).
    pub return_type: Option<String>,
}

/// Parse transition and function declarations from Leo source.
///
/// Detects both `transition` and `function` keywords and extracts
/// parameter information, including record-typed parameters.
pub fn parse_transitions(source: &str) -> Vec<TransitionInfo> {
    let mut transitions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;

        let (is_transition, after_keyword) = if trimmed.starts_with("transition ") {
            (true, trimmed.strip_prefix("transition ").unwrap())
        } else if trimmed.starts_with("function ") {
            // Skip if inside an Aleo program block (not Leo syntax).
            // Leo functions use `function` keyword too.
            (false, trimmed.strip_prefix("function ").unwrap())
        } else {
            continue;
        };

        // Parse: `name(param1: type1, param2: type2) -> ReturnType {`
        let paren_start = after_keyword.find('(');
        let name = if let Some(pos) = paren_start {
            after_keyword[..pos].trim().to_string()
        } else {
            // No parentheses — might be a declaration like `function name:`
            let name_end = after_keyword
                .find(|c: char| c == ':' || c == '{' || c.is_whitespace())
                .unwrap_or(after_keyword.len());
            after_keyword[..name_end].trim().to_string()
        };

        if name.is_empty() {
            continue;
        }

        // Parse parameters.
        let mut params = Vec::new();
        if let Some(paren_start) = paren_start {
            let rest = &after_keyword[paren_start + 1..];
            if let Some(paren_end) = rest.find(')') {
                let params_str = &rest[..paren_end];
                for param in params_str.split(',') {
                    let param = param.trim();
                    if param.is_empty() {
                        continue;
                    }
                    if let Some(colon_pos) = param.find(':') {
                        let pname = param[..colon_pos].trim().to_string();
                        let ptype = param[colon_pos + 1..].trim().to_string();
                        if !pname.is_empty() && !ptype.is_empty() {
                            params.push((pname, ptype));
                        }
                    }
                }
            }
        }

        // Parse return type.
        let return_type = if let Some(arrow_pos) = after_keyword.find("->") {
            let after_arrow = &after_keyword[arrow_pos + 2..];
            let ret = after_arrow
                .trim()
                .trim_end_matches('{')
                .trim()
                .to_string();
            if ret.is_empty() { None } else { Some(ret) }
        } else {
            None
        };

        transitions.push(TransitionInfo {
            name,
            is_transition,
            line: line_num,
            params,
            return_type,
        });
    }

    transitions
}

/// Resolve a record field access to its value.
///
/// Given a record instance and a field name, returns the field value.
/// Handles built-in fields (`owner`, `gates`) and custom fields.
pub fn resolve_record_field(record: &RecordType, field_name: &str) -> Option<String> {
    match field_name {
        "owner" => Some(record.owner.clone()),
        "gates" => Some(record.gates.to_string()),
        _ => {
            // Look up in custom fields.
            record
                .fields
                .iter()
                .find(|(name, _)| name == field_name)
                .map(|(_, value)| value.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Transition tracer
// ---------------------------------------------------------------------------

/// Tracer for transition function execution, including record handling.
pub struct TransitionTracer {
    /// Accumulated trace events.
    events: Vec<TransitionTraceEvent>,
    /// The execution context (caller/signer).
    context: TransitionContext,
    /// Named record instances available in scope.
    records: HashMap<String, RecordType>,
}

impl TransitionTracer {
    /// Create a new transition tracer with the given context.
    pub fn new(context: TransitionContext) -> Self {
        Self {
            events: Vec::new(),
            context,
            records: HashMap::new(),
        }
    }

    /// Return the collected trace events.
    pub fn events(&self) -> &[TransitionTraceEvent] {
        &self.events
    }

    /// Consume the tracer and return all collected events.
    pub fn into_events(self) -> Vec<TransitionTraceEvent> {
        self.events
    }

    /// Return a reference to the execution context.
    pub fn context(&self) -> &TransitionContext {
        &self.context
    }

    /// Register a record instance in the tracer's scope.
    pub fn register_record(&mut self, var_name: &str, record: RecordType) {
        self.records.insert(var_name.to_string(), record);
    }

    /// Emit a transition entry event.
    pub fn enter_transition(&mut self, function_name: &str, is_transition: bool) {
        self.events.push(TransitionTraceEvent::TransitionEntry {
            function_name: function_name.to_string(),
            is_transition,
        });
    }

    /// Resolve a built-in and emit a trace event.
    pub fn trace_builtin(&mut self, name: &str) -> Option<String> {
        let value = resolve_builtin(name, &self.context)?;
        self.events.push(TransitionTraceEvent::BuiltinResolved {
            name: name.to_string(),
            value: value.clone(),
        });
        Some(value)
    }

    /// Access a record field and emit a trace event.
    pub fn trace_record_field_access(
        &mut self,
        record_name: &str,
        field_name: &str,
    ) -> Option<String> {
        let record = self.records.get(record_name)?;
        let value = resolve_record_field(record, field_name)?;
        self.events
            .push(TransitionTraceEvent::RecordFieldAccess {
                record_name: record_name.to_string(),
                field_name: field_name.to_string(),
                value: value.clone(),
            });
        Some(value)
    }

    /// Emit a record output event.
    pub fn trace_record_output(&mut self, record: &RecordType) {
        self.events.push(TransitionTraceEvent::RecordOutput {
            record_type: record.name.clone(),
            owner: record.owner.clone(),
            gates: record.gates,
            fields: record.fields.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Record parsing tests --

    #[test]
    fn test_parse_record_types_single() {
        let source = r#"program token.aleo {
    record token {
        owner: address,
        gates: u64,
        amount: u64,
    }

    transition mint(owner: address, amount: u64) -> token {
        return token {
            owner: owner,
            gates: 0u64,
            amount: amount,
        };
    }
}"#;
        let records = parse_record_types(source);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "token");
        assert_eq!(records[0].fields.len(), 3);
        assert_eq!(records[0].fields[0], ("owner".to_string(), "address".to_string()));
        assert_eq!(records[0].fields[1], ("gates".to_string(), "u64".to_string()));
        assert_eq!(records[0].fields[2], ("amount".to_string(), "u64".to_string()));
    }

    #[test]
    fn test_parse_record_types_multiple() {
        let source = r#"program multi.aleo {
    record coin {
        owner: address,
        gates: u64,
        value: u64,
    }

    record nft {
        owner: address,
        gates: u64,
        token_id: u64,
        metadata: u64,
    }
}"#;
        let records = parse_record_types(source);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].name, "coin");
        assert_eq!(records[0].fields.len(), 3);
        assert_eq!(records[1].name, "nft");
        assert_eq!(records[1].fields.len(), 4);
    }

    #[test]
    fn test_parse_record_types_empty_source() {
        let records = parse_record_types("program empty.aleo { }");
        assert!(records.is_empty());
    }

    // -- Transition detection tests --

    #[test]
    fn test_parse_transitions_detects_transition_keyword() {
        let source = r#"program test.aleo {
    transition mint(owner: address, amount: u64) -> token {
        return token { owner: owner, gates: 0u64, amount: amount };
    }

    transition transfer(sender: token, receiver: address, amount: u64) -> (token, token) {
        return (remaining, transferred);
    }

    function helper() -> u32 {
        return 42u32;
    }
}"#;
        let transitions = parse_transitions(source);
        assert_eq!(transitions.len(), 3);

        // First: transition mint
        assert_eq!(transitions[0].name, "mint");
        assert!(transitions[0].is_transition);
        assert_eq!(transitions[0].params.len(), 2);
        assert_eq!(transitions[0].params[0], ("owner".to_string(), "address".to_string()));
        assert_eq!(transitions[0].params[1], ("amount".to_string(), "u64".to_string()));
        assert_eq!(transitions[0].return_type.as_deref(), Some("token"));

        // Second: transition transfer
        assert_eq!(transitions[1].name, "transfer");
        assert!(transitions[1].is_transition);
        assert_eq!(transitions[1].params.len(), 3);
        assert_eq!(transitions[1].params[0].1, "token");

        // Third: function helper
        assert_eq!(transitions[2].name, "helper");
        assert!(!transitions[2].is_transition);
        assert!(transitions[2].params.is_empty());
    }

    #[test]
    fn test_parse_transitions_with_record_params() {
        let source = r#"program token.aleo {
    record token {
        owner: address,
        gates: u64,
        amount: u64,
    }

    transition transfer(input_record: token, to: address, amount: u64) -> (token, token) {
        let remaining: u64 = input_record.amount - amount;
        return (sender_record, receiver_record);
    }
}"#;
        let transitions = parse_transitions(source);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].name, "transfer");
        assert!(transitions[0].is_transition);
        assert_eq!(transitions[0].params[0], ("input_record".to_string(), "token".to_string()));
    }

    // -- Built-in resolution tests --

    #[test]
    fn test_resolve_builtin_caller() {
        let ctx = TransitionContext::new(
            "aleo1caller000000000000000000000000000000000000000000000000000",
            "aleo1signer000000000000000000000000000000000000000000000000000",
        );
        assert_eq!(
            resolve_builtin("self.caller", &ctx),
            Some("aleo1caller000000000000000000000000000000000000000000000000000".to_string())
        );
    }

    #[test]
    fn test_resolve_builtin_signer() {
        let ctx = TransitionContext::new(
            "aleo1caller000000000000000000000000000000000000000000000000000",
            "aleo1signer000000000000000000000000000000000000000000000000000",
        );
        assert_eq!(
            resolve_builtin("self.signer", &ctx),
            Some("aleo1signer000000000000000000000000000000000000000000000000000".to_string())
        );
    }

    #[test]
    fn test_resolve_builtin_unknown() {
        let ctx = TransitionContext::default_test();
        assert_eq!(resolve_builtin("self.unknown", &ctx), None);
        assert_eq!(resolve_builtin("foo", &ctx), None);
    }

    // -- Record field access tests --

    #[test]
    fn test_resolve_record_field_owner() {
        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1owner".to_string(),
            gates: 0,
            fields: vec![("amount".to_string(), 100)],
        };
        assert_eq!(
            resolve_record_field(&record, "owner"),
            Some("aleo1owner".to_string())
        );
    }

    #[test]
    fn test_resolve_record_field_gates() {
        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1owner".to_string(),
            gates: 500,
            fields: vec![],
        };
        assert_eq!(
            resolve_record_field(&record, "gates"),
            Some("500".to_string())
        );
    }

    #[test]
    fn test_resolve_record_field_custom() {
        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1owner".to_string(),
            gates: 0,
            fields: vec![
                ("amount".to_string(), 1000),
                ("token_id".to_string(), 42),
            ],
        };
        assert_eq!(
            resolve_record_field(&record, "amount"),
            Some("1000".to_string())
        );
        assert_eq!(
            resolve_record_field(&record, "token_id"),
            Some("42".to_string())
        );
    }

    #[test]
    fn test_resolve_record_field_missing() {
        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1owner".to_string(),
            gates: 0,
            fields: vec![],
        };
        assert_eq!(resolve_record_field(&record, "nonexistent"), None);
    }

    // -- TransitionTracer tests --

    #[test]
    fn test_transition_tracer_entry() {
        let ctx = TransitionContext::default_test();
        let mut tracer = TransitionTracer::new(ctx);
        tracer.enter_transition("mint", true);

        let events = tracer.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TransitionTraceEvent::TransitionEntry {
                function_name,
                is_transition,
            } => {
                assert_eq!(function_name, "mint");
                assert!(*is_transition);
            }
            other => panic!("expected TransitionEntry, got {:?}", other),
        }
    }

    #[test]
    fn test_transition_tracer_builtin_resolution() {
        let ctx = TransitionContext::new("aleo1caller", "aleo1signer");
        let mut tracer = TransitionTracer::new(ctx);

        let caller = tracer.trace_builtin("self.caller");
        assert_eq!(caller, Some("aleo1caller".to_string()));

        let signer = tracer.trace_builtin("self.signer");
        assert_eq!(signer, Some("aleo1signer".to_string()));

        let unknown = tracer.trace_builtin("self.unknown");
        assert_eq!(unknown, None);

        // Should have 2 BuiltinResolved events (unknown didn't produce one).
        let builtin_count = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, TransitionTraceEvent::BuiltinResolved { .. }))
            .count();
        assert_eq!(builtin_count, 2);
    }

    #[test]
    fn test_transition_tracer_record_field_access() {
        let ctx = TransitionContext::default_test();
        let mut tracer = TransitionTracer::new(ctx);

        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1owner".to_string(),
            gates: 100,
            fields: vec![("amount".to_string(), 500)],
        };
        tracer.register_record("my_token", record);

        let owner = tracer.trace_record_field_access("my_token", "owner");
        assert_eq!(owner, Some("aleo1owner".to_string()));

        let amount = tracer.trace_record_field_access("my_token", "amount");
        assert_eq!(amount, Some("500".to_string()));

        let gates = tracer.trace_record_field_access("my_token", "gates");
        assert_eq!(gates, Some("100".to_string()));

        // 3 RecordFieldAccess events.
        let field_count = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, TransitionTraceEvent::RecordFieldAccess { .. }))
            .count();
        assert_eq!(field_count, 3);
    }

    #[test]
    fn test_transition_tracer_record_output() {
        let ctx = TransitionContext::default_test();
        let mut tracer = TransitionTracer::new(ctx);

        let record = RecordType {
            name: "token".to_string(),
            owner: "aleo1receiver".to_string(),
            gates: 0,
            fields: vec![("amount".to_string(), 250)],
        };
        tracer.trace_record_output(&record);

        let events = tracer.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TransitionTraceEvent::RecordOutput {
                record_type,
                owner,
                gates,
                fields,
            } => {
                assert_eq!(record_type, "token");
                assert_eq!(owner, "aleo1receiver");
                assert_eq!(*gates, 0);
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0], ("amount".to_string(), 250));
            }
            other => panic!("expected RecordOutput, got {:?}", other),
        }
    }

    #[test]
    fn test_transition_tracer_full_flow() {
        // Simulate a complete transition execution trace.
        let ctx = TransitionContext::new("aleo1alice", "aleo1alice");
        let mut tracer = TransitionTracer::new(ctx);

        // Enter transition.
        tracer.enter_transition("transfer", true);

        // Input record.
        let input_record = RecordType {
            name: "token".to_string(),
            owner: "aleo1alice".to_string(),
            gates: 0,
            fields: vec![("amount".to_string(), 1000)],
        };
        tracer.register_record("input", input_record);

        // Access self.caller.
        let caller = tracer.trace_builtin("self.caller");
        assert_eq!(caller, Some("aleo1alice".to_string()));

        // Access record fields.
        let amount = tracer.trace_record_field_access("input", "amount");
        assert_eq!(amount, Some("1000".to_string()));

        let owner = tracer.trace_record_field_access("input", "owner");
        assert_eq!(owner, Some("aleo1alice".to_string()));

        // Output records.
        let sender_record = RecordType {
            name: "token".to_string(),
            owner: "aleo1alice".to_string(),
            gates: 0,
            fields: vec![("amount".to_string(), 700)],
        };
        tracer.trace_record_output(&sender_record);

        let receiver_record = RecordType {
            name: "token".to_string(),
            owner: "aleo1bob".to_string(),
            gates: 0,
            fields: vec![("amount".to_string(), 300)],
        };
        tracer.trace_record_output(&receiver_record);

        // Verify event counts.
        let events = tracer.events();
        assert_eq!(events.len(), 6); // 1 entry + 1 builtin + 2 field access + 2 output

        // Verify event types in order.
        assert!(matches!(&events[0], TransitionTraceEvent::TransitionEntry { .. }));
        assert!(matches!(&events[1], TransitionTraceEvent::BuiltinResolved { .. }));
        assert!(matches!(&events[2], TransitionTraceEvent::RecordFieldAccess { .. }));
        assert!(matches!(&events[3], TransitionTraceEvent::RecordFieldAccess { .. }));
        assert!(matches!(&events[4], TransitionTraceEvent::RecordOutput { .. }));
        assert!(matches!(&events[5], TransitionTraceEvent::RecordOutput { .. }));
    }
}
