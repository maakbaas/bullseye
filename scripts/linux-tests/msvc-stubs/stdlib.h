/* Only for scripts/linux-tests/clippy-windows.sh: what ring's C code needs, never linked. */
#pragma once
#include <stddef.h>
void abort(void);
unsigned long _byteswap_ulong(unsigned long);
unsigned long long _byteswap_uint64(unsigned long long);
unsigned short _byteswap_ushort(unsigned short);
