// Exercises a runtime abort inside an `async finalize` block.
//
// The transition emits an async future via `then finalize(...)`,
// the finalize handler calls `Mapping::get(balances, caller)` --
// the variant WITHOUT a `get_or_use` default fallback.  On the
// real chain that aborts the finalize when the key is absent;
// the structured evaluator models the same semantics by treating
// every `Mapping::get` as a non-existent-key access (the recorder
// does not model an in-memory mapping store) and surfacing a
// `LeoFinalizeAbort` `EventLogKind::Error` io_event.  The
// finalize frame's CloseFrame marks the frame as failed (no
// post-abort statements are evaluated, no return value
// emitted).
//
// Canonical answer:
//   deposit(amount) returns reported = amount (the future-emitting
//   transition completes successfully on the off-chain side).
//   finalize_deposit(caller, amount) calls Mapping::get(balances,
//   caller) -- ABORTS here, the post-abort `prior + amount`
//   binding is not reached, no return value is emitted.
program async_finalize_failure_test.aleo {
    mapping balances: u32 => u32;

    async transition deposit(amount: u32) -> u32 {
        let reported: u32 = amount;
        return reported;
    }

    async function finalize_deposit(caller: u32, amount: u32) -> u32 {
        let prior: u32 = Mapping::get(balances, caller);
        let updated: u32 = prior + amount;
        Mapping::set(balances, caller, updated);
        return updated;
    }

    transition main() -> u32 {
        let deposited: u32 = deposit(50u32);
        let finalized: u32 = finalize_deposit(7u32, deposited);
        return finalized;
    }
}
