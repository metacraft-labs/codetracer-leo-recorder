// Exercises Leo's integer arithmetic modes near the u32 boundary:
// checked (default `+` -- aborts on overflow), wrapping
// (`add_wrapped`), and saturating (`add_saturating`).
//
// The structured evaluator surfaces each operation distinctly:
//
//   - checked overflow surfaces as `EventLogKind::Error` io_event
//     with metadata `LeoOverflow` (parallel to the `LeoAssert`
//     io_event from M10).  The wrapping result is still bound so
//     downstream arithmetic stays well-defined.
//   - `a.add_wrapped(b)` completes silently with the wrapping
//     result.
//   - `a.add_saturating(b)` returns `u32::MAX` when the true sum
//     would overflow, surfaces nothing extra.
//
// Canonical answer with u32 boundary inputs:
//   max - 1 == 4_294_967_294
//   wrapped = (max - 1).add_wrapped(2u32)        -> 0u32
//   saturated = (max - 1).add_saturating(2u32)   -> 4_294_967_295
//   bad = (max - 1) + 2u32                       -> 0u32 (wrapped)
//                                                + LeoOverflow io_event
//   safe = 1u32 + 2u32                           -> 3u32 (no overflow, no event)
program int_overflow_modes_test.aleo {
    transition compute() -> u32 {
        let near_max: u32 = 4294967294u32;
        let two: u32 = 2u32;
        let wrapped: u32 = near_max.add_wrapped(two);
        let saturated: u32 = near_max.add_saturating(two);
        let bad: u32 = near_max + two;
        let safe: u32 = 1u32 + two;
        return safe;
    }

    transition main() -> u32 {
        return compute();
    }
}
