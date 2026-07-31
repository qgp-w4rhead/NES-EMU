/*
 * fds_audio.h - Famicom Disk System expansion audio (wavetable + modulator).
 * Port of src/mappers/fds_audio.rs to C (M4.4).
 */
#ifndef NES_CORE_C_FDS_AUDIO_H
#define NES_CORE_C_FDS_AUDIO_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FDS_WAVE_TABLE_SIZE 64
#define FDS_MOD_TABLE_SIZE  64

typedef struct FdsAudio {
    uint8_t wave_ram[FDS_WAVE_TABLE_SIZE];
    uint8_t wave_addr;
    uint8_t master_volume;
    bool    env_disabled;
    bool    env_increase;
    uint8_t volume_gain;
    uint16_t freq;
    uint32_t phase_acc;
    uint16_t mod_freq;
    uint32_t mod_phase_acc;
    uint8_t mod_wave[FDS_MOD_TABLE_SIZE];
    uint8_t mod_wave_addr;
    bool    mod_disabled;
    int8_t  mod_gain;
    uint8_t mod_sweep_counter;
    uint8_t mod_sweep_neg;
    uint8_t mod_sweep_pos;
    uint8_t mod_gain_output;
} FdsAudio;

void fds_audio_init(FdsAudio* a);
void fds_audio_write_register(FdsAudio* a, uint16_t addr, uint8_t value);
uint8_t fds_audio_read_register(const FdsAudio* a, uint16_t addr);
void fds_audio_clock(FdsAudio* a, uint32_t apu_cycles);
float fds_audio_sample(const FdsAudio* a);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_FDS_AUDIO_H */
