/* PirateWave Extension ABI 1, Windows x64, C calling convention. MIT licensed.
   All pointers borrowed for the call; no exceptions may cross this boundary.
   See ABI.md for limits, audio layout, threading and ownership. */
#ifndef PIRATEWAVE_EXTENSION_ABI_H
#define PIRATEWAVE_EXTENSION_ABI_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#define PIRATEWAVE_EXTENSION_ABI_VERSION 1u
#define PIRATEWAVE_EXTENSION_MAX_STATE 65536u
typedef struct PirateWaveExtensionApi {
    uint32_t version;
    uint32_t size;
    const uint8_t* (*describe)(uint32_t* length);
    void* (*create)(uint32_t type_index, float sample_rate, uint32_t block_size,
                    uint32_t state_schema, const uint8_t* state, uint32_t state_length);
    int32_t (*migrate)(uint32_t type_index, uint32_t old_schema,
                       const uint8_t* state, uint32_t length,
                       uint8_t* output_65536_bytes, uint32_t* output_length);
    void (*destroy)(void* instance);
    int32_t (*set_parameter)(void* instance, uint32_t parameter_index, float value);
    int32_t (*reset)(void* instance);
    int32_t (*process)(void* instance, const float* const* inputs, uint32_t input_count,
                       float* const* outputs, uint32_t output_count, uint32_t frames);
} PirateWaveExtensionApi;
/* Export this symbol with __declspec(dllexport) from your DLL. */
const PirateWaveExtensionApi* piratewave_extension_api(void);
/* Legacy ABI 1 entry point remains accepted by the host. */
#ifdef __cplusplus
}
#endif
#endif
