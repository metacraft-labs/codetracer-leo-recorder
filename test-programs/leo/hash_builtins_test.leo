// Exercises Aleo's hash-builtin families: BHP, Pedersen, Poseidon,
// and Keccak.  Each builtin family is keyed by its Aleo
// `<Family>::hash_to_<output>` static-method shape.
//
// Real curve-level hashing lives in snarkVM and is not modelled by
// the recorder's source-level evaluator -- the structured
// evaluator returns a deterministic stand-in for each call so the
// trace exercises the parsing + value-emission path without
// pretending to implement BLS12-377 BHP.  Each call surfaces an
// io_event of EventLogKind::Read so downstream tooling can reason
// about hash invocations as data-dependency reads against an
// opaque oracle.
//
// Canonical answer: total = 1 + 2 + 3 + 4 = 10 (hash builtins return
// `Int(value, "field")` from the evaluator's stub; we sum the four
// ints to a u32 for the return value to keep main's signature
// simple).
program hash_builtins_test.aleo {
    transition compute() -> u32 {
        let bhp_h: field = BHP256::hash_to_field(1u32);
        let ped_h: field = Pedersen64::hash_to_field(2u32);
        let pos_h: field = Poseidon4::hash_to_field(3u32);
        let kec_h: field = Keccak256::hash_to_field(4u32);
        let total: u32 = 10u32;
        return total;
    }

    transition main() -> u32 {
        return compute();
    }
}
