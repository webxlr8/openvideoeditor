/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 16:9
 * duration: 12s
 * tracks: 2 · items: 3
 *
 * media:
 *   name       id      type   duration  resolution  status
 *   recorder   m_rec   audio  20s       -           probed
 *   interview  m_talk  video  20s       -           probed
 * cast: (none)
 * diagnostics: (none)
 * pending: 0
 * rev: 2
 */
export default (
  <timeline aspect="16:9">
    <track id="t_v" name="Main" kind="video">
      <clip id="i_v" start={0} end={10} src="interview" denoise={70} />
    </track>
    <track id="t_a" name="Audio" kind="audio">
      <audio id="i_a" start={0} end={10} src="recorder" denoise={100} />
      <audio id="i_off" start={10} end={12} src="recorder" />
    </track>
  </timeline>
);
