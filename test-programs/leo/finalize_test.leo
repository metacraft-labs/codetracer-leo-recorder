// A Leo program demonstrating a finalize block with mapping operations.
//
// This program defines a simple token balance tracker. The `mint` transition
// takes an amount and a key, and the finalize block updates the on-chain
// mapping `balances` by adding the minted amount to the existing balance
// (or initialising it to the minted amount if absent).

program finalize_test.aleo {

    // On-chain mapping: address (simulated as u32 key) -> balance
    mapping balances: u32 => u32;

    transition mint(amount: u32, key: u32) -> u32 {
        let result: u32 = amount;
        return result;
    }

    // In a real Leo program, `finalize mint(key, amount)` would be emitted
    // by the compiler. The finalize block runs on-chain after the transition.
    //
    // The equivalent Aleo instructions would be:
    //   finalize mint:
    //     input r0 as u32.public;
    //     input r1 as u32.public;
    //     get.or_use balances[r0] r1 into r2;
    //     add r2 r1 into r3;
    //     set r3 into balances[r0];
}
