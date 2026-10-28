These are synthetic test fixtures created for this project, with a blue 64×64 frame
and a 440 Hz sine wave, H.264 video and AAC audio. Each playlist is three seconds
long with three one-second segments. No third-party video or audio is used.

Generated with FFmpeg (not required to run tests):

```sh
ffmpeg -f lavfi -i color=c=blue:s=64x64:r=10:d=3 -f lavfi -i sine=frequency=440:sample_rate=44100:duration=3 -c:v libx264 -preset ultrafast -g 10 -c:a aac -b:a 32k -hls_time 1 -hls_list_size 0 -hls_segment_filename ts-%d.ts ts.m3u8
ffmpeg -f lavfi -i color=c=blue:s=64x64:r=10:d=3 -f lavfi -i sine=frequency=440:sample_rate=44100:duration=3 -c:v libx264 -preset ultrafast -g 10 -c:a aac -b:a 32k -hls_time 1 -hls_list_size 0 -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4 -hls_segment_filename mp4-%d.m4s mp4.m3u8
```

Set `NDM_HLS_TEST_OUTPUT_DIR` to keep merged outputs for optional `ffprobe` or
playback inspection. Tests otherwise remove their temporary output files.
