// Exercises Aleo's multi-entry-point dispatch model: a program may
// expose several public `transition` entry points and Aleo callers
// invoke each one independently (deposit / withdraw / query are
// the canonical token-vault triple).
//
// The recorder's structured evaluator historically walked only the
// `main` transition's call graph; without `main` the legacy
// fallback ran a single arbitrary function.  The M11 R3 work
// extends the evaluator so that, when no `main` is declared, every
// top-level `transition` is executed in declaration order as its
// own entry point.  Each surfaces its own Function entry with a
// balanced Call/Return pair.
//
// Canonical answers:
//   deposit(7u32, 100u32):
//     credited = balance + amount = 107
//     return credited            -> 107
//   withdraw(50u32, 30u32):
//     debited = balance - amount = 20
//     return debited             -> 20
//   query(42u32):
//     view = balance + 1u32      = 43
//     return view                -> 43
program multi_test_entry_test.aleo {
    transition deposit(balance: u32, amount: u32) -> u32 {
        let credited: u32 = balance + amount;
        return credited;
    }

    transition withdraw(balance: u32, amount: u32) -> u32 {
        let debited: u32 = balance - amount;
        return debited;
    }

    transition query(balance: u32) -> u32 {
        let view: u32 = balance + 1u32;
        return view;
    }
}
