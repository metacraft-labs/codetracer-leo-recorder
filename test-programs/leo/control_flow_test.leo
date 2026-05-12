// Exercises Leo's control-flow surfaces: if/else, ternary, and a
// compile-time bounded `for` loop.  Control flow is an essential
// category in the universal recorder-test checklist
// (`metacraft-specs/policies/recorder-test-requirements.md` §2).
//
// `compute()` walks an if/else cascade keyed off `raw`, applies a
// `?:` ternary to derive `bonus`, and finally accumulates a
// `for i in 0u32..3u32` loop into `result`.
//
// The Leo source is fully spec-compliant.  Whether the Leo recorder
// today decodes each individual let-binding -- or only the linear
// arithmetic ones the source-level parser understands -- is asserted
// in `tests/test_tracer.rs::test_control_flow_test_via_ct_print_full`
// and the parallel `#[ignore]`d expectation captures the spec-
// correct values.
program control_flow_test.aleo {
    transition compute() -> u32 {
        let raw: u32 = 7u32;
        let sign: u32 = if raw > 5u32 { 1u32 } else { 2u32 };
        let bonus: u32 = sign == 1u32 ? 300u32 : 100u32;
        let combined: u32 = sign + bonus;
        let mut result: u32 = combined;
        for i: u32 in 0u32..3u32 {
            result = result + i;
        }
        return result;
    }

    transition main() -> u32 {
        return compute();
    }
}
