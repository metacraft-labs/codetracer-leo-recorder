// Bare `.aleo` dialect fixture: hand-written Aleo instructions
// (the form the Leo compiler emits) exercised directly by the
// recorder.  The Leo source-level structured evaluator does not
// know about register-based instructions, so the recorder
// auto-detects the `.aleo` extension and routes through a
// dedicated synthesiser that lowers each instruction into a step
// event surfacing the destination register's post-instruction
// value.
//
// `cast r3 into r4 as u64` re-types r3's payload as u64; the
// output r4 carries the post-cast type id (distinct from r3's
// u32 id) so downstream tooling can reconstruct the AVM-level
// type chain.
//
// Inputs default to 5 (r0) and 6 (r1) -- the recorder's bare-
// `.aleo` synthesiser seeds private inputs with the
// declaration-order index plus a small base (5) so each register
// surfaces a distinct, non-zero value.
//
// Canonical answer:
//   r0 = 5                       (input)
//   r1 = 6                       (input)
//   r2 = r0 + r1 = 11            (add)
//   r3 = r2 * 2u32 = 22          (mul)
//   r4 = r3 as u64 = 22          (cast)
//   output r4 as u64.private     (final value, type u64)
program compute_aleo.aleo;

function compute:
    input r0 as u32.private;
    input r1 as u32.private;
    add r0 r1 into r2;
    mul r2 2u32 into r3;
    cast r3 into r4 as u64;
    output r4 as u64.private;
