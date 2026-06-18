#[allow(dead_code)]
pub fn exit() -> ! {
    use cortex_m::asm::bkpt;

    loop {
        bkpt();
    }
}
