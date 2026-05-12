// Exercises Leo's `assert` / `assert_eq` surfaces on a passing path.
//
// Each assertion holds, so execution runs to completion and returns
// the expected sum.  This complements `error_paths_test.leo` (which
// drives the failing branch) by covering the spec-required "assert
// with handler" cell of the universal-checklist's exceptions/errors
// row.  Today the recorder doesn't surface `assert` calls as either
// step events or explicit `RecordEvent`s — only the surrounding let-
// bindings round-trip; see RECORDER BUG notes in
// `tests/test_tracer.rs::test_assert_test_via_ct_print_full`.
program assert_test.aleo {
    transition compute() -> u32 {
        let a: u32 = 4u32;
        let b: u32 = 5u32;
        assert_eq(a + b, 9u32);
        let sum_val: u32 = a + b;
        assert(sum_val > 0u32);
        return sum_val;
    }

    transition main() -> u32 {
        return compute();
    }
}
