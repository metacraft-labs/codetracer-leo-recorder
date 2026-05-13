// Exercises Aleo's UTXO primitive: `record` types and the canonical
// mint/transfer/burn lifecycle.  A `record` is a struct-like
// value with an `owner` field carrying the holder's address; on
// the real chain, mint creates a new UTXO, transfer consumes one
// and produces a new one, and burn consumes a UTXO without
// producing a successor.
//
// The recorder treats each record as a `ValueRecord::Struct`
// (owner is surfaced as a u32 stand-in for the address since the
// source-level evaluator does not model addresses as a distinct
// value kind); each lifecycle transition surfaces as a regular
// call_entry/call_exit pair with the produced record value
// flowing through the call's return value.
//
// Canonical answer:
//   mint(99, 100) -> Token { owner: 99, amount: 100 }
//   transfer(.., 200, 80) -> Token { owner: 200, amount: 80 }
//   burn(..) -> 80   (the amount field of the burnt record)
//   main() -> 80
program record_token_test.aleo {
    struct Token {
        owner: u32,
        amount: u32,
    }

    function mint(owner: u32, amount: u32) -> Token {
        let minted: Token = Token { owner: owner, amount: amount };
        return minted;
    }

    function transfer(input: Token, receiver: u32, send: u32) -> Token {
        let leftover: u32 = input.amount - send;
        let new_token: Token = Token { owner: receiver, amount: send };
        let _residual: u32 = leftover;
        return new_token;
    }

    function burn(input: Token) -> u32 {
        let consumed: u32 = input.amount;
        return consumed;
    }

    transition compute() -> u32 {
        let initial: Token = mint(99u32, 100u32);
        let moved: Token = transfer(initial, 200u32, 80u32);
        let burnt: u32 = burn(moved);
        return burnt;
    }

    transition main() -> u32 {
        return compute();
    }
}
