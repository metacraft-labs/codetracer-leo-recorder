//! Finalize-scope instruction-level tracing for Aleo programs.
//!
//! Provides `FinalizeTracer`, which wraps the AVM interpreter to emit
//! a trace event for **each** Aleo instruction executed (not just per
//! variable binding). This is essential for the M4 milestone:
//! per-instruction trace capture during finalize execution, including
//! Mapping commands (get, get_or_use, set, remove, contains).

use std::collections::HashMap;

use crate::tracer::{
    AleoFunction, AleoInstruction, FunctionResult, MappingStore, Operand, resolve_operand,
};
use eyre::{Result, eyre};

// ---------------------------------------------------------------------------
// Trace events emitted by FinalizeTracer
// ---------------------------------------------------------------------------

/// A single trace event produced during finalize-scope execution.
#[derive(Debug, Clone)]
pub enum FinalizeTraceEvent {
    /// A step event: one Aleo instruction was executed.
    Step {
        /// The function name containing the instruction.
        function_name: String,
        /// 0-based instruction index within the function.
        instruction_index: usize,
        /// Textual description of the instruction.
        instruction_text: String,
    },
    /// A variable/register event: register contents after an instruction.
    Variable {
        /// Register name (e.g. "r0").
        name: String,
        /// Register value.
        value: i64,
    },
}

// ---------------------------------------------------------------------------
// FinalizeTracer
// ---------------------------------------------------------------------------

/// Per-instruction tracer for Aleo finalize-scope execution.
///
/// Unlike the main `LeoTracer` which emits events per Leo source binding,
/// `FinalizeTracer` emits a `Step` event for every single Aleo instruction
/// and `Variable` events showing register contents after each instruction.
pub struct FinalizeTracer {
    /// Accumulated trace events.
    events: Vec<FinalizeTraceEvent>,
    /// Mapping store for finalize-scope mapping operations.
    mapping_store: MappingStore,
}

impl FinalizeTracer {
    /// Create a new `FinalizeTracer` with an empty mapping store.
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            mapping_store: MappingStore::new(),
        }
    }

    /// Create a new `FinalizeTracer` with a pre-populated mapping store.
    pub fn with_mapping_store(mapping_store: MappingStore) -> Self {
        Self {
            events: Vec::new(),
            mapping_store,
        }
    }

    /// Return a reference to the collected trace events.
    pub fn events(&self) -> &[FinalizeTraceEvent] {
        &self.events
    }

    /// Consume the tracer and return all collected events.
    pub fn into_events(self) -> Vec<FinalizeTraceEvent> {
        self.events
    }

    /// Return a reference to the mapping store.
    pub fn mapping_store(&self) -> &MappingStore {
        &self.mapping_store
    }

    /// Execute a function with per-instruction tracing.
    ///
    /// For each instruction executed, emits:
    /// 1. A `Step` event identifying the instruction.
    /// 2. `Variable` events for every register that has been written so far.
    pub fn trace_function(
        &mut self,
        func: &AleoFunction,
        func_map: &HashMap<&str, &AleoFunction>,
        input_values: &[i64],
    ) -> Result<FunctionResult> {
        let mut registers: HashMap<usize, i64> = HashMap::new();

        // Set input registers.
        for (idx, &(reg, _)) in func.inputs.iter().enumerate() {
            if idx < input_values.len() {
                registers.insert(reg, input_values[idx]);
            } else {
                registers.insert(reg, 0);
            }
        }

        // Execute each instruction with tracing.
        for (instr_idx, instr) in func.instructions.iter().enumerate() {
            // Execute the instruction.
            self.execute_instruction(
                instr,
                &mut registers,
                func,
                func_map,
                instr_idx,
            )?;
        }

        // Collect outputs.
        let outputs: Vec<i64> = func
            .outputs
            .iter()
            .map(|&reg| registers.get(&reg).copied().unwrap_or(0))
            .collect();

        Ok(FunctionResult { registers, outputs })
    }

    /// Execute a single instruction, emitting trace events.
    fn execute_instruction(
        &mut self,
        instr: &AleoInstruction,
        registers: &mut HashMap<usize, i64>,
        func: &AleoFunction,
        func_map: &HashMap<&str, &AleoFunction>,
        instr_idx: usize,
    ) -> Result<()> {
        let instr_text = format_instruction(instr);

        // Emit Step event.
        self.events.push(FinalizeTraceEvent::Step {
            function_name: func.name.clone(),
            instruction_index: instr_idx,
            instruction_text: instr_text,
        });

        // Execute the instruction.
        match instr {
            AleoInstruction::Add { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, v1.wrapping_add(v2));
            }
            AleoInstruction::Sub { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, v1.wrapping_sub(v2));
            }
            AleoInstruction::Mul { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, v1.wrapping_mul(v2));
            }
            AleoInstruction::Div { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                if v2 != 0 {
                    registers.insert(*dest, v1 / v2);
                } else {
                    return Err(eyre!("division by zero"));
                }
            }
            AleoInstruction::Mod { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                if v2 != 0 {
                    registers.insert(*dest, v1 % v2);
                } else {
                    return Err(eyre!("modulo by zero"));
                }
            }
            AleoInstruction::IsEq { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 == v2 { 1 } else { 0 });
            }
            AleoInstruction::IsNeq { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 != v2 { 1 } else { 0 });
            }
            AleoInstruction::Lt { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 < v2 { 1 } else { 0 });
            }
            AleoInstruction::Lte { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 <= v2 { 1 } else { 0 });
            }
            AleoInstruction::Gt { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 > v2 { 1 } else { 0 });
            }
            AleoInstruction::Gte { src1, src2, dest } => {
                let v1 = resolve_operand(src1, registers);
                let v2 = resolve_operand(src2, registers);
                registers.insert(*dest, if v1 >= v2 { 1 } else { 0 });
            }
            AleoInstruction::Call {
                function_name,
                args,
                dests,
            } => {
                let arg_values: Vec<i64> =
                    args.iter().map(|op| resolve_operand(op, registers)).collect();

                if let Some(callee) = func_map.get(function_name.as_str()) {
                    let callee_result =
                        self.trace_function(callee, func_map, &arg_values)?;
                    for (idx, &dest) in dests.iter().enumerate() {
                        if idx < callee_result.outputs.len() {
                            registers.insert(dest, callee_result.outputs[idx]);
                        }
                    }
                }
            }
            AleoInstruction::MappingGet {
                mapping,
                key_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), registers);
                let key_str = key.to_string();
                let value = self.mapping_store.get(mapping, &key_str).unwrap_or(0);
                registers.insert(*value_reg, value);
            }
            AleoInstruction::MappingGetOrUse {
                mapping,
                key_reg,
                default_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), registers);
                let default = resolve_operand(&Operand::Register(*default_reg), registers);
                let key_str = key.to_string();
                let value = self.mapping_store.get_or_use(mapping, &key_str, default);
                registers.insert(*value_reg, value);
            }
            AleoInstruction::MappingSet {
                mapping,
                key_reg,
                value_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), registers);
                let value = resolve_operand(&Operand::Register(*value_reg), registers);
                let key_str = key.to_string();
                self.mapping_store.set(mapping, &key_str, value);
            }
            AleoInstruction::MappingRemove { mapping, key_reg } => {
                let key = resolve_operand(&Operand::Register(*key_reg), registers);
                let key_str = key.to_string();
                self.mapping_store.remove(mapping, &key_str);
            }
            AleoInstruction::MappingContains {
                mapping,
                key_reg,
                result_reg,
            } => {
                let key = resolve_operand(&Operand::Register(*key_reg), registers);
                let key_str = key.to_string();
                let result = if self.mapping_store.contains(mapping, &key_str) {
                    1
                } else {
                    0
                };
                registers.insert(*result_reg, result);
            }
            AleoInstruction::Unknown(_) => {
                // Skip.
            }
        }

        // Emit Variable events for all registers after this instruction.
        let mut reg_indices: Vec<usize> = registers.keys().copied().collect();
        reg_indices.sort();
        for reg_idx in reg_indices {
            if let Some(&val) = registers.get(&reg_idx) {
                self.events.push(FinalizeTraceEvent::Variable {
                    name: format!("r{}", reg_idx),
                    value: val,
                });
            }
        }

        Ok(())
    }
}

/// Format an `AleoInstruction` into a human-readable string.
fn format_instruction(instr: &AleoInstruction) -> String {
    match instr {
        AleoInstruction::Add { src1, src2, dest } => {
            format!("add {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Sub { src1, src2, dest } => {
            format!("sub {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Mul { src1, src2, dest } => {
            format!("mul {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Div { src1, src2, dest } => {
            format!("div {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Mod { src1, src2, dest } => {
            format!("mod {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::IsEq { src1, src2, dest } => {
            format!("is.eq {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::IsNeq { src1, src2, dest } => {
            format!("is.neq {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Lt { src1, src2, dest } => {
            format!("lt {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Lte { src1, src2, dest } => {
            format!("lte {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Gt { src1, src2, dest } => {
            format!("gt {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Gte { src1, src2, dest } => {
            format!("gte {} {} into r{}", format_operand(src1), format_operand(src2), dest)
        }
        AleoInstruction::Call { function_name, args, dests } => {
            let args_str: Vec<String> = args.iter().map(format_operand).collect();
            let dests_str: Vec<String> = dests.iter().map(|d| format!("r{}", d)).collect();
            if dests_str.is_empty() {
                format!("call {} {}", function_name, args_str.join(" "))
            } else {
                format!("call {} {} into {}", function_name, args_str.join(" "), dests_str.join(" "))
            }
        }
        AleoInstruction::MappingGet { mapping, key_reg, value_reg } => {
            format!("get {}[r{}] into r{}", mapping, key_reg, value_reg)
        }
        AleoInstruction::MappingGetOrUse { mapping, key_reg, default_reg, value_reg } => {
            format!("get.or_use {}[r{}] r{} into r{}", mapping, key_reg, default_reg, value_reg)
        }
        AleoInstruction::MappingSet { mapping, key_reg, value_reg } => {
            format!("set r{} into {}[r{}]", value_reg, mapping, key_reg)
        }
        AleoInstruction::MappingRemove { mapping, key_reg } => {
            format!("remove {}[r{}]", mapping, key_reg)
        }
        AleoInstruction::MappingContains { mapping, key_reg, result_reg } => {
            format!("contains {}[r{}] into r{}", mapping, key_reg, result_reg)
        }
        AleoInstruction::Unknown(s) => s.clone(),
    }
}

/// Format an operand for display.
fn format_operand(op: &Operand) -> String {
    match op {
        Operand::Register(idx) => format!("r{}", idx),
        Operand::Literal(val) => format!("{}", val),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracer::parse_aleo_program;

    #[test]
    fn test_finalize_tracer_arithmetic() {
        let aleo_source = r#"program test.aleo;

finalize compute:
    input r0 as u32.public;
    add r0 10u32 into r1;
    mul r1 2u32 into r2;
    output r2 as u32.public;
"#;
        let functions = parse_aleo_program(aleo_source);
        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut tracer = FinalizeTracer::new();
        let result = tracer
            .trace_function(func_map["compute"], &func_map, &[5])
            .unwrap();

        // r0=5 (input), r1=15 (5+10), r2=30 (15*2)
        assert_eq!(result.registers[&0], 5);
        assert_eq!(result.registers[&1], 15);
        assert_eq!(result.registers[&2], 30);
        assert_eq!(result.outputs, vec![30]);

        // Should have 2 Step events (2 instructions, not counting input/output).
        let step_count = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, FinalizeTraceEvent::Step { .. }))
            .count();
        assert_eq!(step_count, 2);

        // Each step should be followed by Variable events for all registers.
        let var_count = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, FinalizeTraceEvent::Variable { .. }))
            .count();
        // After instruction 0 (add): 2 regs (r0, r1) => 2 Variable events
        // After instruction 1 (mul): 3 regs (r0, r1, r2) => 3 Variable events
        assert_eq!(var_count, 2 + 3);
    }

    #[test]
    fn test_finalize_tracer_mapping_operations() {
        // Simulate a finalize block that uses mapping operations.
        let functions = vec![AleoFunction {
            name: "update_balance".to_string(),
            is_closure: false,
            inputs: vec![(0, "u32".to_string()), (1, "u32".to_string())],
            instructions: vec![
                // set r1 into balances[r0]
                AleoInstruction::MappingSet {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    value_reg: 1,
                },
                // contains balances[r0] into r2
                AleoInstruction::MappingContains {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    result_reg: 2,
                },
                // get balances[r0] into r3
                AleoInstruction::MappingGet {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    value_reg: 3,
                },
            ],
            outputs: vec![3],
        }];

        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut tracer = FinalizeTracer::new();
        // key=42, value=100
        let result = tracer
            .trace_function(func_map["update_balance"], &func_map, &[42, 100])
            .unwrap();

        // After set: balances[42] = 100
        // After contains: r2 = 1 (true)
        // After get: r3 = 100
        assert_eq!(result.registers[&2], 1);
        assert_eq!(result.registers[&3], 100);
        assert_eq!(result.outputs, vec![100]);

        // 3 instructions => 3 Step events.
        let step_count = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, FinalizeTraceEvent::Step { .. }))
            .count();
        assert_eq!(step_count, 3);
    }

    #[test]
    fn test_finalize_tracer_get_or_use() {
        let functions = vec![AleoFunction {
            name: "safe_read".to_string(),
            is_closure: false,
            inputs: vec![(0, "u32".to_string()), (1, "u32".to_string())],
            instructions: vec![
                // get.or_use balances[r0] r1 into r2
                AleoInstruction::MappingGetOrUse {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    default_reg: 1,
                    value_reg: 2,
                },
            ],
            outputs: vec![2],
        }];

        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        // Key does not exist, should return default.
        let mut tracer = FinalizeTracer::new();
        let result = tracer
            .trace_function(func_map["safe_read"], &func_map, &[99, 500])
            .unwrap();
        assert_eq!(result.registers[&2], 500); // default value

        // Now pre-populate the mapping and try again.
        let mut store = MappingStore::new();
        store.set("balances", "99", 777);
        let mut tracer2 = FinalizeTracer::with_mapping_store(store);
        let result2 = tracer2
            .trace_function(func_map["safe_read"], &func_map, &[99, 500])
            .unwrap();
        assert_eq!(result2.registers[&2], 777); // existing value
    }

    #[test]
    fn test_finalize_tracer_mapping_remove() {
        let functions = vec![AleoFunction {
            name: "remove_entry".to_string(),
            is_closure: false,
            inputs: vec![(0, "u32".to_string())],
            instructions: vec![
                // remove balances[r0]
                AleoInstruction::MappingRemove {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                },
                // contains balances[r0] into r1
                AleoInstruction::MappingContains {
                    mapping: "balances".to_string(),
                    key_reg: 0,
                    result_reg: 1,
                },
            ],
            outputs: vec![1],
        }];

        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut store = MappingStore::new();
        store.set("balances", "10", 999);

        let mut tracer = FinalizeTracer::with_mapping_store(store);
        let result = tracer
            .trace_function(func_map["remove_entry"], &func_map, &[10])
            .unwrap();

        // After remove, contains should return 0.
        assert_eq!(result.registers[&1], 0);
        assert_eq!(result.outputs, vec![0]);
    }

    #[test]
    fn test_finalize_tracer_step_events_per_instruction() {
        // Verify that every instruction produces exactly one Step event
        // and that Variable events follow each Step.
        let functions = vec![AleoFunction {
            name: "multi".to_string(),
            is_closure: false,
            inputs: vec![],
            instructions: vec![
                AleoInstruction::Add {
                    src1: Operand::Literal(1),
                    src2: Operand::Literal(2),
                    dest: 0,
                },
                AleoInstruction::Add {
                    src1: Operand::Literal(3),
                    src2: Operand::Literal(4),
                    dest: 1,
                },
                AleoInstruction::Add {
                    src1: Operand::Register(0),
                    src2: Operand::Register(1),
                    dest: 2,
                },
            ],
            outputs: vec![2],
        }];

        let func_map: HashMap<&str, &AleoFunction> =
            functions.iter().map(|f| (f.name.as_str(), f)).collect();

        let mut tracer = FinalizeTracer::new();
        let result = tracer
            .trace_function(func_map["multi"], &func_map, &[])
            .unwrap();

        assert_eq!(result.registers[&0], 3);
        assert_eq!(result.registers[&1], 7);
        assert_eq!(result.registers[&2], 10);

        // 3 Step events.
        let steps: Vec<_> = tracer
            .events()
            .iter()
            .filter(|e| matches!(e, FinalizeTraceEvent::Step { .. }))
            .collect();
        assert_eq!(steps.len(), 3);

        // Verify Step events carry instruction indices 0, 1, 2.
        for (i, step) in steps.iter().enumerate() {
            if let FinalizeTraceEvent::Step {
                instruction_index, ..
            } = step
            {
                assert_eq!(*instruction_index, i);
            }
        }
    }

    #[test]
    fn test_parse_mapping_instructions() {
        use crate::tracer::parse_aleo_instruction;

        // get
        let instr = parse_aleo_instruction("get balances[r0] into r1;").unwrap();
        match instr {
            AleoInstruction::MappingGet {
                mapping,
                key_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(value_reg, 1);
            }
            other => panic!("expected MappingGet, got {:?}", other),
        }

        // get.or_use
        let instr = parse_aleo_instruction("get.or_use balances[r0] r2 into r3;").unwrap();
        match instr {
            AleoInstruction::MappingGetOrUse {
                mapping,
                key_reg,
                default_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(default_reg, 2);
                assert_eq!(value_reg, 3);
            }
            other => panic!("expected MappingGetOrUse, got {:?}", other),
        }

        // set
        let instr = parse_aleo_instruction("set r1 into balances[r0];").unwrap();
        match instr {
            AleoInstruction::MappingSet {
                mapping,
                key_reg,
                value_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(value_reg, 1);
            }
            other => panic!("expected MappingSet, got {:?}", other),
        }

        // remove
        let instr = parse_aleo_instruction("remove balances[r0];").unwrap();
        match instr {
            AleoInstruction::MappingRemove { mapping, key_reg } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
            }
            other => panic!("expected MappingRemove, got {:?}", other),
        }

        // contains
        let instr = parse_aleo_instruction("contains balances[r0] into r1;").unwrap();
        match instr {
            AleoInstruction::MappingContains {
                mapping,
                key_reg,
                result_reg,
            } => {
                assert_eq!(mapping, "balances");
                assert_eq!(key_reg, 0);
                assert_eq!(result_reg, 1);
            }
            other => panic!("expected MappingContains, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_aleo_finalize_block() {
        let aleo_source = r#"program token.aleo;

mapping balances:
    key as address.public;
    value as u64.public;

function transfer:
    input r0 as address.private;
    input r1 as u64.private;
    output r0 as address.private;

finalize transfer:
    input r0 as address.public;
    input r1 as u64.public;
    get.or_use balances[r0] r1 into r2;
    add r2 r1 into r3;
    set r3 into balances[r0];
"#;
        let functions = parse_aleo_program(aleo_source);

        // Should parse both the function and the finalize block.
        assert_eq!(functions.len(), 2);

        // Both are named "transfer" but one is the function (with output) and
        // one is the finalize (with mapping instructions).
        // Since both have name "transfer", find the one with 3 instructions.
        let finalize = functions.iter().find(|f| f.instructions.len() == 3).unwrap();
        assert_eq!(finalize.instructions.len(), 3);
    }
}
