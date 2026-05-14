// Exercises Aleo's Schnorr signature primitive: a `signature`
// value carries the (r, s) scalar pair and the
// `sig.verify(signer, msg) -> bool` instance method evaluates the
// Schnorr verification equation against the on-chain signer
// address and the signed message.
//
// Real Schnorr verification lives in snarkVM and depends on the
// chain's group/scalar curve; the recorder's source-level
// evaluator returns a deterministic stand-in (`true`) so the
// trace exercises the parsing + value-emission path without
// pretending to implement Schnorr.  Each call surfaces an
// io_event of `EventLogKind::Read` tagged `LeoSignatureVerify`
// with the verbatim source-level expression so downstream tooling
// can reason about signature checks as data-dependency reads
// against an opaque verifier oracle.
//
// `signature` literals use the bech32m `sign1...` form; `address`
// literals use the canonical `aleo1...` form.  Both surface as
// `Value::Str` with a distinguishing lang_type (`signature` or
// `address`) so the writer registers a `TypeKind::String` id that
// downstream tooling can use to disambiguate from generic strings.
//
// Canonical answer:
//   verify_payment(sig, signer, 7field) -> sig.verify(signer, 7field) = true
//   main() -> true
program signature_verify_test.aleo {
    transition verify_payment(sig: signature, signer: address, msg: field) -> bool {
        let ok: bool = sig.verify(signer, msg);
        return ok;
    }

    transition main() -> bool {
        let sig: signature = sign1abcdefghij;
        let signer: address = aleo1qnr4dkkvkgfqph0vzc3y6z2eu975wnpz2925ntjccd76cqsnpdjsdhgg6c;
        let result: bool = verify_payment(sig, signer, 7field);
        return result;
    }
}
