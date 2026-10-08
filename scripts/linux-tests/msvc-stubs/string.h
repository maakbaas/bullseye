/* Only for scripts/linux-tests/clippy-windows.sh: what ring's C code needs, never linked. */
#pragma once
#include <stddef.h>
void *memcpy(void*,const void*,size_t);void *memset(void*,int,size_t);int memcmp(const void*,const void*,size_t);void *memmove(void*,const void*,size_t);
