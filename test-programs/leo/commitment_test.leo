// Exercises Aleo's commitment-scheme builtins: BHP and Pedersen
// `commit_to_*` methods.  Commitments are hash-style functions that
// take a payload + a randomness scalar and produce a hiding,
// binding `field` value.
//
// Real curve-level commitment math lives in snarkVM; the recorder's
// source-level evaluator returns a deterministic stand-in (sum of
// integer arg payloads, typed `field`) so the trace exercises the
// parsing + value-emission path without modelling BLS12-377.
// Each call surfaces an io_event of EventLogKind::Read with a
// family-tagged metadata (`LeoBhpHash` / `LeoPedersenHash`) and
// the verbatim source-level call text.
//
// Canonical answer:
//   commit_secret(7field, 5scalar):
//     bhp_c    = BHP256::commit_to_field(7, 5)     = 12 (sum stub, typed field)
//     ped_c    = Pedersen64::commit_to_field(7, 5) = 12 (sum stub, typed field)
//     return bhp_c
//   reveal_check(12field, 7field, 5scalar):
//     fresh = BHP256::commit_to_field(7, 5) = 12 (sum stub, typed field)
//     ok    = fresh == commitment            = true
//     return ok
program commitment_test.aleo {
    transition commit_secret(secret: field, randomness: scalar) -> field {
        let bhp_c: field = BHP256::commit_to_field(secret, randomness);
        let ped_c: field = Pedersen64::commit_to_field(secret, randomness);
        return bhp_c;
    }

    transition reveal_check(commitment: field, secret: field, randomness: scalar) -> bool {
        let fresh: field = BHP256::commit_to_field(secret, randomness);
        let ok: bool = fresh == commitment;
        return ok;
    }

    transition main() -> bool {
        let c: field = commit_secret(7field, 5scalar);
        let verified: bool = reveal_check(c, 7field, 5scalar);
        return verified;
    }
}
