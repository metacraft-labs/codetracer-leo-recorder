// Exercises Aleo's cross-program call surface: a caller program
// (`program_imports_test.aleo`) imports a library program
// (`math_lib.aleo`) via `import math_lib.aleo;` and invokes the
// library's transition through the qualified call shape
// `math_lib.aleo/double(x)`.
//
// The recorder loads BOTH .leo files from the same directory
// (the caller's directory is the import-resolution root), parses
// them into one merged program, and registers each function's
// source path with the writer so source-line metadata for the
// library function points at `math_lib.leo` rather than the
// caller.
//
// Canonical answer:
//   inner = math_lib.aleo/double(7) = 14
//   doubled = inner + 1 = 15
//   main() returns doubled = 15
import math_lib.aleo;

program program_imports_test.aleo {
    transition compute() -> u32 {
        let inner: u32 = math_lib.aleo/double(7u32);
        let doubled: u32 = inner + 1u32;
        return doubled;
    }

    transition main() -> u32 {
        return compute();
    }
}
