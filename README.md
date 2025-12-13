# RA6M3 RTIC Proof of Concept

This repository is a proof of concept for using the RTIC framework with the
RA6M3 microcontroller. It includes MQTT(S) v5 client and a HTTP server.

IMPORTANT: Run with debugger attached. If not, set logger channel to `NoBlockTrim`
instead of `BlockIfFull`, or it won't work.
