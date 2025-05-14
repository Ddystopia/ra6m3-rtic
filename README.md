# RA6M3 RTIC Proof of Concept

This repository is a proof of concept for using the RTIC framework with the
RA6M3 microcontroller. It includes MQTT(S) v5 client and a HTTP server.

## How to run

Application logic can be executed in qemu based on `lm3s6965` microcontroller.
You need to set `CC` in `.cargo/config.toml` pointing to cortex_m3-eabi gcc, as
well as configure other env variables. Ensure correct `build.target` field.

```bash
go-task run-debug-qemu
```

This command will setup the network and run the application in qemu. You can
configure other env variables in `.cargo/config.toml`.

## Todo

- Real time metrics
- Cannot reconnect to MQTT broker after it resets
