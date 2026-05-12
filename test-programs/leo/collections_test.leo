// Exercises Leo's structured value kinds: struct, tuple-return, and
// fixed-size array (Leo's only sized collection in transition
// scope).  Source is spec-shaped; the recorder is expected to surface
// the array as a `ValueRecord::Sequence`, the (u32,u32) tuple return
// as `ValueRecord::Tuple`, and the `Point` struct as
// `ValueRecord::Struct`.  Today only the integer let-bindings round-
// trip; see RECORDER BUG notes in `tests/test_tracer.rs`.
program collections_test.aleo {
    struct Point {
        x: u32,
        y: u32,
    }

    function point_distance_sq(p: Point) -> u32 {
        let xx: u32 = p.x * p.x;
        let yy: u32 = p.y * p.y;
        let dist_sq: u32 = xx + yy;
        return dist_sq;
    }

    function sum_pair(p: (u32, u32)) -> u32 {
        return p.0 + p.1;
    }

    function array_total() -> u32 {
        let xs: [u32; 4] = [1u32, 2u32, 3u32, 4u32];
        let head_val: u32 = xs[0u32];
        let len: u32 = 4u32;
        let total: u32 = head_val + len;
        return total;
    }

    transition compute() -> u32 {
        let pair_total: u32 = sum_pair((10u32, 20u32));
        let pt_total: u32 = point_distance_sq(Point { x: 3u32, y: 4u32 });
        let array_total_val: u32 = array_total();
        let grand_total: u32 = pair_total + pt_total + array_total_val;
        return grand_total;
    }

    transition main() -> u32 {
        return compute();
    }
}
