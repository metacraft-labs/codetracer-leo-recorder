program flow_test.aleo {
    transition compute() -> u32 {
        let a: u32 = 10u32;
        let b: u32 = 32u32;
        let sum_val: u32 = a + b;
        let doubled: u32 = sum_val * 2u32;
        let final_result: u32 = doubled + a;
        return final_result;
    }

    transition main() -> u32 {
        return compute();
    }
}
