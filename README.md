# RA6M3 RTIC Proof of Concept

This repository is a proof of concept for using the RTIC framework with the
RA6M3 microcontroller. It includes MQTT v5 client and a HTTPS server.

## How to run

Application logic can be executed in qemu based on `lm3s6965` microcontroller.
You need to set `CC` in `.cargo/config.toml` pointing to cortex_m3-eabi gcc.

```bash
go-task run-debug-qemu
```

This command will setup the network and run the application in qemu.
You can configure other env variables in `.cargo/config.toml`.

## Todo

- RA6M3 support
- TLS
- Real time metrics
