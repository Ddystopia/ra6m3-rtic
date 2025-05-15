use ra_fsp_rs::ioport::{
    IoPortConfig, e_bsp_io_port_pin_t::*, e_ioport_cfg_options::*, e_ioport_peripheral::*,
    ioport_cfg_t, ioport_pin_cfg_t,
};

pub static BSP_PIN_CFG: ioport_cfg_t = IoPortConfig::new(&[
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_00,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_01,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_02,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_04,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_08,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_09,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_00_PIN_14,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_00,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_01,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_02,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_03,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_04,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_06,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_07,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_08,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_DEBUG,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_09,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_DEBUG,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_10,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_DEBUG,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_11,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_12,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_13,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_14,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_01_PIN_15,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_02,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_03,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_04,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_05,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_06,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT | IOPORT_CFG_PULLUP_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_08,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_TRACE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_09,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_TRACE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_10,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_TRACE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_11,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_TRACE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_12,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_CLKOUT_COMP_RTC,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_13,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_CLKOUT_COMP_RTC,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_02_PIN_14,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_TRACE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_00,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_DEBUG,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_01,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_02,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_03,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_04,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_HIGH,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_05,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_06,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_07,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_08,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_09,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_03_PIN_10,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_QSPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_00,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_01,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_02,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_03,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_04,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_HIGH,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_05,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_06,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_07,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_FS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_08,
        pin_cfg: IOPORT_CFG_NMOS_ENABLE
            | IOPORT_CFG_PERIPHERAL_PIN
            | IOPORT_PERIPHERAL_SCI1_3_5_7_9,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_09,
        pin_cfg: IOPORT_CFG_NMOS_ENABLE
            | IOPORT_CFG_PERIPHERAL_PIN
            | IOPORT_PERIPHERAL_SCI1_3_5_7_9,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_10,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_11,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_12,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_13,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_14,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SPI,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_04_PIN_15,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_GPT1,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_00,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_FS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_01,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_FS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_03,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_04,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_05,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_06,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_07,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_08,
        pin_cfg: IOPORT_CFG_ANALOG_ENABLE,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_11,
        pin_cfg: IOPORT_CFG_DRIVE_MID | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_IIC,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_05_PIN_12,
        pin_cfg: IOPORT_CFG_DRIVE_MID | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_IIC,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_00,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_01,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_02,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_03,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_GPT1,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_08,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_09,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_10,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_LCD_GRAPHICS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_11,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_13,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SCI1_3_5_7_9,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_06_PIN_14,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_SCI1_3_5_7_9,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_00,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_01,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_02,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_03,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_04,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_05,
        pin_cfg: IOPORT_CFG_DRIVE_HIGH | IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_ETHER_RMII,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_06,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_07,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_HS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_07_PIN_08,
        pin_cfg: IOPORT_CFG_IRQ_ENABLE | IOPORT_CFG_PORT_DIRECTION_INPUT,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_00,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_01,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_02,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_03,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_04,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_08_PIN_05,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_09_PIN_07,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_09_PIN_08,
        pin_cfg: IOPORT_CFG_PORT_DIRECTION_OUTPUT | IOPORT_CFG_PORT_OUTPUT_LOW,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_11_PIN_00,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_HS,
    },
    ioport_pin_cfg_t {
        pin: BSP_IO_PORT_11_PIN_01,
        pin_cfg: IOPORT_CFG_PERIPHERAL_PIN | IOPORT_PERIPHERAL_USB_HS,
    },
])
.c_conf();
