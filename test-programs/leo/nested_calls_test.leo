// Three-deep call chain: main -> compute -> inner.  This exercises
// the universal "nested calls (>=3 deep)" requirement from
// `metacraft-specs/policies/recorder-test-requirements.md` §2.
//
// Each callee returns through the next caller; the host observes
// 1 + 2 = 3 from the inner-most function and propagates it back
// untouched.
//
// We keep the chain shallow because the recorder's source-level
// parser today resolves call targets via a hand-rolled `return
// callee();` walk and a HashMap-iteration scan that becomes
// non-deterministic — and crashes outright — once two intermediate
// callers both have empty bodies.  See RECORDER BUG notes in
// `tests/test_tracer.rs::test_nested_calls_test_via_ct_print_full`
// and the parallel `#[ignore]`d expectation.
program nested_calls_test.aleo {
    function inner() -> u32 {
        let a: u32 = 1u32;
        let b: u32 = 2u32;
        let c: u32 = a + b;
        return c;
    }

    transition compute() -> u32 {
        return inner();
    }

    transition main() -> u32 {
        return compute();
    }
}
