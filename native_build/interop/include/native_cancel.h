#ifndef NATIVE_CANCEL_H
#define NATIVE_CANCEL_H
#include <stdbool.h>
typedef struct native_cancel native_cancel;
native_cancel *native_cancel_create(void);
void native_cancel_retain(const native_cancel *);
void native_cancel_release(const native_cancel *);
/* Nonblocking request. False means terminal completion already won. */
bool native_cancel_request(const native_cancel *);
bool native_cancel_requested(const native_cancel *);
/* Workflow owner only, after all children and cleanup are quiescent. */
bool native_cancel_finish(const native_cancel *);
#endif
