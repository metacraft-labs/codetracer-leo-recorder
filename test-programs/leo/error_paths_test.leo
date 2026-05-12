// Exercises Leo's error/assert surfaces.
//
// `safe_compute()` runs a normal arithmetic chain so the recorder
// has a baseline of step events to emit before the failing path.
// `failing_compute()` triggers `assert(false)` after computing two
// let-bindings — the recorder must surface those bindings *plus* an
// error event for the failed assertion.  Today the recorder traces
// only the entry path picked up by `main()` and elides assertion
// failures entirely; see RECORDER BUG notes in
// `tests/test_tracer.rs::test_error_paths_test_via_ct_print_full`.
program error_paths_test.aleo {
    function safe_compute() -> u32 {
        let a: u32 = 5u32;
        let b: u32 = 7u32;
        let c: u32 = a + b;
        return c;
    }

    function failing_compute() -> u32 {
        let seen_a: u32 = 1u32;
        let seen_b: u32 = 2u32;
        // Intentional: drives the recorder's error path.
        assert_eq(seen_a, seen_b);
        return seen_a;
    }

    transition compute() -> u32 {
        let safe_val: u32 = safe_compute();
        let bumped: u32 = safe_val + 100u32;
        return bumped;
    }

    transition main() -> u32 {
        return compute();
    }
}
