// Library program imported by `program_imports_test.leo`.
// Exposes a single transition `double(x) -> 2 * x` that the
// caller program invokes via `math_lib.aleo/double(...)`.
program math_lib.aleo {
    transition double(x: u32) -> u32 {
        let doubled: u32 = x + x;
        return doubled;
    }
}
