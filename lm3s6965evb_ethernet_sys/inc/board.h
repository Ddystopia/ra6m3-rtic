#ifndef __BOARD_H__
#define __BOARD_H__

extern void __lm3s6965evb_ethernet_assert(unsigned char, const char *, const char *, unsigned long);

#define assert(x) __lm3s6965evb_ethernet_assert(x, __func__, __FILE__, __LINE__)

#endif // __BOARD_H__
