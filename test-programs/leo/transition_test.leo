program transition_test.aleo {
    record token {
        owner: address,
        gates: u64,
        amount: u64,
    }

    transition mint(owner: address, amount: u64) -> token {
        return token {
            owner: owner,
            gates: 0u64,
            amount: amount,
        };
    }

    transition transfer(input: token, to: address, amount: u64) -> (token, token) {
        let remaining: u64 = input.amount - amount;
        let sender: token = token {
            owner: self.caller,
            gates: 0u64,
            amount: remaining,
        };
        let receiver: token = token {
            owner: to,
            gates: 0u64,
            amount: amount,
        };
        return (sender, receiver);
    }

    transition main() -> u32 {
        let result: u32 = 42u32;
        return result;
    }
}
