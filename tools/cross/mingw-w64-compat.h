/* Force-included into whisper.cpp / ggml when cross-compiling for Windows with MinGW-w64 older than 12 (Ubuntu 24.04
 * ships 11), whose headers lack the thread power-throttling declarations ggml uses to keep its worker threads off
 * Windows 11's throttled cores. Values and layout match the Windows SDK and mingw-w64 12 (processthreadsapi.h).
 * Only <_mingw.h> is included, so no Windows macros (min/max, ...) leak into the sources. */
#ifndef OVE_MINGW_W64_COMPAT_H
#define OVE_MINGW_W64_COMPAT_H
#ifdef __MINGW32__
#include <_mingw.h>
#if __MINGW64_VERSION_MAJOR < 12 && !defined(THREAD_POWER_THROTTLING_CURRENT_VERSION)
#define THREAD_POWER_THROTTLING_CURRENT_VERSION 1
#define THREAD_POWER_THROTTLING_EXECUTION_SPEED 0x1
#define THREAD_POWER_THROTTLING_VALID_FLAGS (THREAD_POWER_THROTTLING_EXECUTION_SPEED)
typedef struct _THREAD_POWER_THROTTLING_STATE {
  unsigned long Version;
  unsigned long ControlMask;
  unsigned long StateMask;
} THREAD_POWER_THROTTLING_STATE;
#endif
#endif
#endif
