"""Build licensed clean/clipped/echo examples for the beginner guide."""
import json
from pathlib import Path
import numpy as np
import soundfile as sf
from assets import sha256

root=Path(__file__).resolve().parents[2]
source=root/'.tools/voice-training/assets/LibriSpeech/dev-clean/1462/170138/1462-170138-0000.flac'
audio,rate=sf.read(source,dtype='float32')
audio=audio[:8*rate]
output=root/'docs/audio'
output.mkdir(exist_ok=True)
sf.write(output/'voice-clean.wav',audio,rate,subtype='PCM_16')
sf.write(output/'voice-clipped.wav',np.clip(audio*6,-.45,.45),rate,subtype='PCM_16')
echo=audio.copy()
for seconds,gain in ((.12,.45),(.24,.3),(.36,.2)):
    offset=int(seconds*rate)
    echo[offset:]+=audio[:-offset]*gain
echo*=min(1,.9/max(float(np.abs(echo).max()),1e-6))
sf.write(output/'voice-room-echo.wav',echo,rate,subtype='PCM_16')
(output/'NOTICES.txt').write_text('LibriSpeech dev-clean, clip 1462-170138-0000. Source: https://www.openslr.org/12 . Vassil Panayotov and Daniel Povey, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). First eight seconds excerpted; clipped and echo versions modified by PirateWave for teaching. Source SHA-256: '+sha256(source)+'\n',encoding='utf-8')
