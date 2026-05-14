// Exercises Leo's explicit type-cast operator (` as <type>`).
// Casts widen, narrow (with truncation), or convert across the
// curve type families (`field` / `scalar` / `group`).
//
// The structured evaluator computes each cast, registers the
// target type id, and surfaces the truncated payload when a
// narrowing cast loses bits (so e.g. `u32(300) as u8 == 44`,
// matching the `300 % 256` semantics of an Aleo trunc cast).
//
// Canonical answer:
//   small  = 100u32
//   big    = small as u64           -> 100u64
//   narrow = (300u32) as u8         -> 44u8 (300 mod 256)
//   f      = big as field           -> 100 (typed `field`)
//   s      = f as scalar            -> 100 (typed `scalar`)
//   total  = small + (narrow as u32) -> 100 + 44 = 144
program cast_test.aleo {
    transition compute() -> u32 {
        let small: u32 = 100u32;
        let big: u64 = small as u64;
        let wide: u32 = 300u32;
        let narrow: u8 = wide as u8;
        let f: field = big as field;
        let s: scalar = f as scalar;
        let total: u32 = small + (narrow as u32);
        return total;
    }

    transition main() -> u32 {
        return compute();
    }
}
