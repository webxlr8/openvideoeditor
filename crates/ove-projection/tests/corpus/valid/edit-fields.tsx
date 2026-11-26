/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 16:9
 * duration: 8s
 * tracks: 3 · items: 3
 *
 * media:
 *   name    id     type   duration  resolution  status
 *   camera  m_cam  video  20s       -           probed
 * cast: (none)
 * diagnostics: (none)
 * pending: 0
 * rev: 2
 */
export default (
  <timeline aspect="16:9" fps={25}>
    <track id="t_v1" name="V1" kind="video">
      <clip id="i_v" start={0} end={4} src="camera" link="L1" muted={true} />
    </track>
    <track id="t_a1" name="A1" kind="audio">
      <audio id="i_a" start={0} end={4} src="camera" link="L1" />
    </track>
    <track id="t_music" name="Music" kind="audio" syncLock={false}>
      <audio id="i_m" start={0} end={8} in={2.04} src="camera" />
    </track>
  </timeline>
);
