// Three-deep helper chain exercising the structured evaluator's
// call propagation: main -> outer -> middle -> inner.
//
// Each helper consumes argumentful inputs and returns its
// computation, so the recorder must round-trip parameter binding
// (formal name + value) at every call site, propagate the inner
// return up through the chain, and surface every let-binding in
// per-call source order.
//
// Canonical answer:
//   inner(2u32, 3u32) -> 2 + 3 + 1 = 6
//   middle(4u32) -> let m = inner(4 - 2, 4 - 1) = inner(2, 3) = 6;
//                   return m + 4 = 10
//   outer(7u32) -> let o = middle(7 - 3) = middle(4) = 10;
//                  return o + 5 = 15
//   main() -> outer(7u32) = 15
program multi_function_test.aleo {
    function inner(x: u32, y: u32) -> u32 {
        let s: u32 = x + y;
        let bumped: u32 = s + 1u32;
        return bumped;
    }

    function middle(v: u32) -> u32 {
        let inner_val: u32 = inner(v - 2u32, v - 1u32);
        let m: u32 = inner_val + v;
        return m;
    }

    function outer(z: u32) -> u32 {
        let middle_val: u32 = middle(z - 3u32);
        let o: u32 = middle_val + 5u32;
        return o;
    }

    transition main() -> u32 {
        return outer(7u32);
    }
}
