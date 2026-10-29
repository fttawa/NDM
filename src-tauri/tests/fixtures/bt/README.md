These synthetic torrents contain no third-party media. Both describe `sample/a.bin`
(70,001 bytes from `hashlib.shake_256(b'NDM BEP52 fixture')`) and `sample/b.bin`
(`b'NDM BTv2\n'`, 9 bytes), created with libtorrent 2.1.1 and 32 KiB pieces.
`v2.torrent` uses `create_torrent.v2_only`; `hybrid.torrent` uses flags 0.
The creation timestamp is omitted. No trackers or external resources are used.

The v2 content total is 70,010 bytes. The v1 hybrid view additionally includes
alignment padding. These fixtures exercise BEP 52 parsing, engine routing,
cached metadata, and task-owned file deletion.
