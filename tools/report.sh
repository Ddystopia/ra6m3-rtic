#!/bin/sh

NM=/opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm
OBJDUMP=/opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-objdump
DEFAULT_IMAGE=target/thumbv7em-none-eabi/debug/project

# Check if the first argument is a valid file path
if [ -f "$1" ]; then
  IMAGE=$1
  shift
else
  IMAGE=$DEFAULT_IMAGE
fi

human_readable_size () {
  num=$1
  if [ $num -lt 1024 ]; then
    echo "${num} B"
  elif [ $num -lt $((10 * 1024 * 1024)) ]; then
    echo "$((num / 1024)) KB"
  elif [ $num -lt $((10 * 1024 * 1024 * 1024)) ]; then
    echo "$((num / (1024 * 1024))) MB"
  else
    echo "$((num / (10 * 1024 * 1024 * 1024))) GB"
  fi
}

# print section headers
# no params
section_headers () {
  ${OBJDUMP} -h build/${IMAGE}
}

# print symbols ordered by size
# args: <filter>
symbols_by_size () {
  ${NM} -C --size-sort -S ${IMAGE} | awk '
  {
    size_hex = $2;
    size_dec = strtonum("0x" size_hex) / 1024;
    name = substr($4, 1, 100);
    if (length($4) > 100) {
      name = name "[...cutted...]";
    }
    printf "%10s %10s %8.3f KB   %s   %s\n", $1, $2, size_dec, $3, name;
  }'
}

# filter to calculate total size of symbols from stdin
size_total () {
  awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
}

filter_flash () {
  grep -Ew '[tTW]'
}

filter_ram () {
  grep -iw b
}

# <n>
filter_top () {
  tac | head -n $1
}

case "$1" in
  section_headers)          section_headers ;;
  flash_symbols_by_size)    symbols_by_size | filter_flash ;;
  ram_symbols_by_size)      symbols_by_size | filter_ram ;;
  flash_total)              
    total=$(symbols_by_size | filter_flash | size_total)
    echo "flash total: $total bytes (`human_readable_size $total`)" ;;
  ram_total)
    total=$(symbols_by_size | filter_ram | size_total)
    echo "ram total: $total bytes (`human_readable_size $total`)" ;;
  *)
    n_top=20
    flash_total=$(symbols_by_size | filter_flash | size_total)
    echo "flash total: $flash_total bytes (`human_readable_size $flash_total`)"
    echo "flash top $n_top:"
    symbols_by_size | filter_flash | filter_top $n_top
    ram_total=$(symbols_by_size | filter_ram | size_total)
    echo "ram total: $ram_total bytes (`human_readable_size $ram_total`)"
    echo "ram top $n_top:"
    symbols_by_size | filter_ram | filter_top $n_top
esac
#/opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-objdump -h ${IMAGE}

 #    5477  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | vim -
 # 5478  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | hx -
 # 5479  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5480  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | awk '{print "0x"$2}' | grep _nx | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5481  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | grep _nx | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5482  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | grep _gx | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5483  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | grep _tx | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5484  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | grep :: | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5485  ccmake .
 # 5486  make clean
 # 5487  makej
 # 5488  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | grep :: | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"
 # 5489  /opt/arm-none-eabi-gcc-11.2.0/bin/arm-bare_newlib_cortex_m4f_nommu-eabihf-nm -C --size-sort -S ${IMAGE} | grep -iw t | awk '{print "0x"$2}' | python -c "import sys; print(sum(int(l,16) for l in sys.stdin))"

