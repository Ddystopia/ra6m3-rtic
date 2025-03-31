# RA6M3 RTIC Proof of Concept

This repository is a proof of concept for using the RTIC framework with the
RA6M3 microcontroller. It includes MQTT v5 client and a HTTPS server.

## How to run

Application logic can be executed in qemu based on `lm3s6965` microcontroller.
You need to set `CC` in `.cargo/config.toml` pointing to cortex_m3-eabi gcc.

```bash
go-task run-debug-qemu
```

This command will setup the network and run the application in qemu. You can
configure other env variables in `.cargo/config.toml`.

## Todo

- RA6M3 support
- TLS
- Real time metrics

## Notes on RA6M3 hal

I think that we should rewrite `SystemInit` but don't recurse into other
funtions. Other function should be ffi called from rust, because it would be a
burden to support.

So, `src/hal_entry.c` and `fsp` itself should be compiled in `build.rs` of the
`hal` crate. To avoid conflicts where the same struct is transiled multiple
times to `Rust`, I don't want to expose `fsp` directly, only through the
wrappers - even if `unsafe`.

Note: I don't know the origin for `src/hal_entry.c`. May we need a separate one
for each board? What is setting me off is that `BSP_WARM_START_POST_CLOCK` is
not handled.

### What would we do?

- Out function will be called after `cortex-m-rt` does it's job

### `cortex-m-rt`

- Expose `Reset` symbol
- (optionally) init stack pointer
- (optionally) init vector table
- sets MSPLIM to \_stack_end
- calls `__pre_init`. This function cannot be Rust
- (optionally) init the whole ram
- (optionally) paint the stack
- initit ebss and sbss
- init data section
- enable fpu if present
- call `main`
- traps if `main` returns

https://github.com/rust-embedded/cortex-m/blob/898b11c7783246fe0137537d20898b91e58ff6aa/cortex-m-rt/src/lib.rs#L524

### FSP

Has lots of conditional compilation, but in our case:

(checkbox means that cortex-m-rt alredy does that)

- [x] Enables FPU
- [x] Sets vector table
- [ ] Calls `R_BSP_WarmStart(BSP_WARM_START_RESET)` ( No C runtime environment,
      clocks, or IRQs ) does nothing here
- [ ] Calls `bsp_clock_init` to configure clocks
- [ ] Calls `R_BSP_WarmStart(BSP_WARM_START_POST_CLOCK)` (Called after clock
      initialization. No C runtime environment or IRQs) does nothing here
- [ ] Setups SPMON->{CTL, OAD, SA, EA, CTL}, R_ICU->NMIER, read the source at
      systemc:323. In pac seems to be
      `https://docs.rs/ra6m3/latest/ra6m3/spmon/struct.RegisterBlock.html`
- [x] Zeroes bss
- [x] Copies initialized RAM data from ROM to RAM.
- ~~[ ] Initializes tls with `_init_tls(&__tls_base);` and
      `_set_tls(&__tls_base);`.~~ Not sure what this does, I am not able to find
      those functions.
- ~~[ ] Initializes static constructors (calls pointer from **init_array_start
      to **init_array_end)~~. Rust nor C does not have static constructors.
- [ ] Calls `SystemCoreClockUpdate`
- [ ] Calls `R_BSP_Init_RTC`
- [ ] Calls `R_BSP_WarmStart(BSP_WARM_START_POST_C)`

* [ ] Calls `R_IOPORT_Open` to configure pins.

- [ ] Calls `bsp_irq_cfg` to initialize ELC events that will be used to trigger
      NVIC interrupts.
- [ ] Calls `bsp_init(NULL);` - I did not find any implementation for this
      function.

### `bsp_irq_cfg`

```c
for (uint32_t i = 0U; i < (BSP_ICU_VECTOR_MAX_ENTRIES - BSP_FEATURE_ICU_FIXED_IELSR_COUNT); i++)
{
    if (0U != g_interrupt_event_link_select[i])
    {
        R_ICU->IELSR[i] = (uint32_t) g_interrupt_event_link_select[i];
    }
}

```

### `R_BSP_Init_RTC`

```c
R_RTC->RCR4 = 0;
R_BSP_RegisterProtectDisable(BSP_REG_PROTECT_OM_LPC_BATT);
R_SYSTEM->VBTICTLR = 0U;
R_BSP_RegisterProtectEnable(BSP_REG_PROTECT_OM_LPC_BATT);
```

### `SystemCoreClockUpdate`

```c
SystemCoreClockUpdate();
    uint32_t clock_index = FSP_STYPE3_REG8_READ(R_SYSTEM->SCKSCR, secure);
        // macro expands to (R_SYSTEM->SCKSCR)
    uint32_t ick =
        (FSP_STYPE3_REG32_READ(R_SYSTEM->SCKDIVCR, secure) & R_SYSTEM_SCKDIVCR_ICK_Msk) >> R_SYSTEM_SCKDIVCR_ICK_Pos;
        // macro expands to (R_SYSTEM->SCKDIVCR)
    SystemCoreClock = g_clock_freq[clock_index] >> ick;
```

### `bsp_clock_init`

In our case (ra6m3):

```c
R_SYSTEM->PRCR = (uint16_t) BSP_PRV_PRCR_UNLOCK;

R_BSP_FlashCacheDisable();
    R_FCACHE->FCACHEE = 0U;

bsp_clock_freq_var_init();
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_HOCO] = BSP_HOCO_HZ;
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_MOCO] = BSP_MOCO_FREQ_HZ;
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_LOCO] = BSP_LOCO_FREQ_HZ;
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_MAIN_OSC] = BSP_CFG_XTAL_HZ;
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_SUBCLOCK] = BSP_SUBCLOCK_FREQ_HZ;
    g_clock_freq[BSP_CLOCKS_SOURCE_CLOCK_PLL] = BSP_STARTUP_SOURCE_CLOCK_HZ;


R_SYSTEM->MOMCR = BSP_PRV_MOMCR;
R_SYSTEM->MOSCWTCR = (uint8_t) BSP_CLOCK_CFG_MAIN_OSC_WAIT;
/* Initialize the sub-clock according to the BSP configuration. */
bsp_prv_sosc_init();
    if (R_SYSTEM->SOSCCR || (BSP_CLOCK_CFG_SUBCLOCK_DRIVE != R_SYSTEM->SOMCR_b.SODRV))
        if (0U == R_SYSTEM->SOSCCR)
            R_SYSTEM->SOSCCR = 1U;
            R_BSP_SoftwareDelay(BSP_PRV_SUBCLOCK_STOP_INTERVAL_US, BSP_DELAY_UNITS_MICROSECONDS);
            FSP_HARDWARE_REGISTER_WAIT(R_SYSTEM->SOSCCR, 1U);
                while (reg != required_value)
        R_SYSTEM->SOMCR =
                ((BSP_CLOCK_CFG_SUBCLOCK_DRIVE << BSP_FEATURE_CGC_SODRV_SHIFT) & BSP_FEATURE_CGC_SODRV_MASK);
        R_SYSTEM->SOSCCR = 0U;
        R_BSP_SubClockStabilizeWaitAfterReset(BSP_CLOCK_CFG_SUBCLOCK_STABILIZATION_MS);
            FSP_PARAMETER_NOT_USED(delay_ms);


R_SYSTEM->HOCOWTCR = BSP_FEATURE_CGC_HOCOWTCR_VALUE;
R_SYSTEM->MOSCCR = 0U;
FSP_HARDWARE_REGISTER_WAIT(R_SYSTEM->OSCSF_b.MOSCSF, 1U);
    while (reg != required_value) { /* Wait. */}
R_SYSTEM->PLLCCR = (uint16_t) BSP_PRV_PLLCCR;
R_SYSTEM->PLLCR = 0U;
FSP_HARDWARE_REGISTER_WAIT(R_SYSTEM->OSCSF_b.PLLSF, 1U);
    while (reg != required_value) { /* Wait. */}
bsp_prv_clock_set_hard_reset();
    R_FCACHE->FLWT = BSP_PRV_ROM_TWO_WAIT_CYCLES;
    R_SYSTEM->SCKDIVCR = BSP_PRV_STARTUP_SCKDIVCR;
    R_SYSTEM->SCKSCR = BSP_CFG_CLOCK_SOURCE;
    SystemCoreClockUpdate();
R_SYSTEM->BCKCR   = BSP_CFG_BCLK_OUTPUT - 1U;
R_SYSTEM->EBCKOCR = 1U;
R_SYSTEM->SDCKOCR = BSP_CFG_SDCLK_OUTPUT;
R_SYSTEM->SCKDIVCR2 = BSP_PRV_UCK_DIV << BSP_PRV_SCKDIVCR2_UCK_BIT;
R_SYSTEM->PRCR = (uint16_t) BSP_PRV_PRCR_LOCK;
R_BSP_FlashCacheEnable();
    R_FCACHE->FCACHEIV = 1U;
    FSP_HARDWARE_REGISTER_WAIT(R_FCACHE->FCACHEIV, 0U);
        while (reg != required_value) { /* Wait. */}

    /* Enable flash cache. */
    R_FCACHE->FCACHEE = 1U;


```

#### R_IOPORT_Open

`R_BSP_WarmStart(BSP_WARM_START_POST_C)` (Called after clocks and C runtime
environment have been set up) calls
`R_IOPORT_Open (&g_ioport_ctrl, g_ioport.p_cfg);` in `hal_entry.c`
