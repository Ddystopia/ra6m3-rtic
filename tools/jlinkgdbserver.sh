#! /bin/sh

jgdbs=$(which JLinkGDBServer)

port=${1:-3333}
rtt_port=${2:-3334}

exec "$jgdbs" -if swd -port "$port" -RTTTelnetPort "$rtt_port" -device R7FA6M3AH -jlinkscriptfile ./rtt.JLinkScript
