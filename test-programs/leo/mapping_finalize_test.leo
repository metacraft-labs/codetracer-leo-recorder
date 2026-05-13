// Exercises Leo's `mapping` ops inside an `async finalize` block.
// The transition emits an async future that the on-chain finalize
// handler consumes; the finalize handler is the only place where
// `Mapping::set` / `Mapping::get_or_use` calls are legal.
//
// The recorder lowers each `Mapping::set` to an io_event with
// `EventLogKind::Write` and each `Mapping::get` /
// `Mapping::get_or_use` to an io_event with `EventLogKind::Read`,
// so a downstream consumer can reconstruct the on-chain state-
// mutation sequence from the trace alone.
//
// Canonical answer: mint reports the requested amount untouched
// (10u32) and finalize reads the prior balance for the receiver
// (default 0u32), adds the minted amount, and writes the updated
// balance back.  main() drives mint() then finalize_mint() so
// both surface in the recorder's call graph.
program mapping_finalize_test.aleo {
    mapping balances: u32 => u32;

    async transition mint(receiver: u32, amount: u32) -> u32 {
        let reported: u32 = amount;
        return reported;
    }

    async function finalize_mint(receiver: u32, amount: u32) -> u32 {
        let prior: u32 = Mapping::get_or_use(balances, receiver, 0u32);
        let updated: u32 = prior + amount;
        Mapping::set(balances, receiver, updated);
        return updated;
    }

    transition main() -> u32 {
        let minted: u32 = mint(7u32, 10u32);
        let finalized: u32 = finalize_mint(7u32, minted);
        return finalized;
    }
}
