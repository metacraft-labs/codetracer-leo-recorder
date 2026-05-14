// Exercises Aleo's parameter visibility modifiers (`public` /
// default-private).  On the real chain, `public` parameters surface
// as on-chain-visible witness inputs (their values are committed to
// transaction metadata and visible to anyone reading the chain),
// whereas the default-private parameters are zero-knowledge
// witnesses that never leave the prover.
//
// The recorder distinguishes the two forms at parameter-emission
// time by appending a `.public` suffix to the type name registered
// with the writer when the parameter carries a leading `public`
// keyword.  Default-private parameters keep their bare type name
// (matches the historical type-name pins used by the rest of the
// suite).  Each variant lands as a distinct `TypeId` so downstream
// tooling can reconstruct the on-chain visibility scheme from the
// trace alone.
//
// Canonical answer:
//   payment(7u64, 100u64, 5u64) = 7 + 100 + 5 = 112
//   main() = payment(7u64, 100u64, 5u64) = 112
program public_private_io_test.aleo {
    transition payment(public sender: u64, public amount: u64, secret: u64) -> u64 {
        let sender_check: u64 = sender;
        let total: u64 = sender_check + amount + secret;
        return total;
    }

    transition main() -> u64 {
        return payment(7u64, 100u64, 5u64);
    }
}
