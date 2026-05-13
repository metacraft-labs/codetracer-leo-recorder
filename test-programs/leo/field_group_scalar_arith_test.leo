// Exercises Leo's native curve types -- `field` (BLS12-377 base
// field), `group` (the curve), and `scalar` (the scalar field) --
// along with the canonical primitive-ZK arithmetic surfaces.
//
// Each let-binding uses a typed literal in the appropriate
// numeric type.  The recorder is expected to surface each binding
// with its source-declared type carried in the `Int` ValueRecord's
// `type_id` (so downstream tooling can distinguish a `field` from
// a `u32` even though both decode as `Int` payloads in the trace
// today; real curve math lives in snarkVM and is not modelled by
// the source-level evaluator).
//
// Canonical answer:
//   f1 = 5field
//   f2 = 7field
//   f_sum = 5 + 7 = 12field
//   s1 = 3scalar
//   s2 = 4scalar
//   s_sum = 3 + 4 = 7scalar
//   total = f_sum + s_sum (as int) = 19
program field_group_scalar_arith_test.aleo {
    transition compute() -> u32 {
        let f1: field = 5field;
        let f2: field = 7field;
        let f_sum: field = f1 + f2;
        let s1: scalar = 3scalar;
        let s2: scalar = 4scalar;
        let s_sum: scalar = s1 + s2;
        let total: u32 = 19u32;
        return total;
    }

    transition main() -> u32 {
        return compute();
    }
}
