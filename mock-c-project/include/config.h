#pragma once

/* Conditional include based on platform.
   Your crawler can record both edges, or branch based on a configured platform. */
#if defined(_WIN32) || defined(_WIN64)
  #include "platform/win.h"
#else
  #include "platform/posix.h"
#endif
