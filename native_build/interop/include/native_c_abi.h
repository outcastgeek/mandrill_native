#ifndef MANDRILL_NATIVE_C_ABI_H
#define MANDRILL_NATIVE_C_ABI_H

#include <stddef.h>
#include <stdint.h>

typedef struct native_abi_header {
    uint32_t abi_version;
    uint32_t struct_size;
} native_abi_header;

#define NATIVE_DECLARE_CONST_SLICE(name, element_type) \
    typedef struct name {                                \
        const element_type *data;                        \
        size_t length;                                   \
    } name

#define NATIVE_DECLARE_MUT_SLICE(name, element_type) \
    typedef struct name {                              \
        element_type *data;                           \
        size_t length;                                \
        size_t capacity;                              \
    } name

#define NATIVE_DECLARE_MAP(name, entry_type) \
    NATIVE_DECLARE_CONST_SLICE(name, entry_type)

#define NATIVE_DECLARE_RESULT(name, value_type) \
    typedef struct name {                       \
        uint32_t status;                        \
        uint32_t reserved;                      \
        value_type value;                       \
    } name

#define NATIVE_DECLARE_INLINE_BYTES(name, capacity_value) \
    typedef struct name {                                 \
        size_t length;                                    \
        uint8_t bytes[capacity_value];                    \
    } name

NATIVE_DECLARE_CONST_SLICE(native_bytes, uint8_t);
NATIVE_DECLARE_CONST_SLICE(native_u32_slice, uint32_t);
NATIVE_DECLARE_CONST_SLICE(native_u64_slice, uint64_t);
NATIVE_DECLARE_MUT_SLICE(native_byte_buffer, uint8_t);
NATIVE_DECLARE_MUT_SLICE(native_u32_buffer, uint32_t);
NATIVE_DECLARE_MUT_SLICE(native_u64_buffer, uint64_t);

typedef void (*native_release_fn)(void *owner);
typedef void (*native_async_callback)(void *context,
                                      uint32_t status,
                                      const void *result,
                                      native_release_fn release,
                                      void *owner);

static inline int native_const_slice_valid(const void *data, size_t length) {
    return length == 0 || data != NULL;
}

static inline int native_mut_slice_valid(const void *data,
                                         size_t length,
                                         size_t capacity) {
    return length <= capacity && (capacity == 0 || data != NULL);
}

#endif
