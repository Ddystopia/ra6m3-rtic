#! /bin/sh

jgdbs=$(which JLinkGDBServer)
$jgdbs -if swd -port 3333 -RTTTelnetPort 3334 -device R7FA6M3AH -jlinkscriptfile ./rtt.JLinkScript
