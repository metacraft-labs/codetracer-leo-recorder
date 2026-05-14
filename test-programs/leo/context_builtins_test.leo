// Exercises Aleo's call-context built-in reads (`self.caller`,
// `self.signer`) and the finalize-scope chain context read
// (`block.height`).
//
// The structured evaluator surfaces `self.caller` and `self.signer`
// as `Value::String` (bech32m address form, stand-in literals so
// the trace is deterministic without depending on a real signing
// key), and `block.height` as `Value::Int` typed `u32`.
//
// Canonical answer:
//   who_am_i() returns the canonical caller stand-in literal so the
//   transition surfaces an address-shaped string in its return
//   value.  finalize_who_am_i() reads block.height and stores it
//   into the `last_seen` mapping under the caller key.
program context_builtins_test.aleo {
    mapping last_seen: address => u32;

    async transition who_am_i() -> address {
        let caller_addr: address = self.caller;
        let signer_addr: address = self.signer;
        return caller_addr;
    }

    async function finalize_who_am_i() -> u32 {
        let height: u32 = block.height;
        Mapping::set(last_seen, self.caller, height);
        return height;
    }

    transition main() -> u32 {
        let _addr: address = who_am_i();
        let recorded: u32 = finalize_who_am_i();
        return recorded;
    }
}
