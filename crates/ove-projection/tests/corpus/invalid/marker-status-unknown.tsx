/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 16:9
 * duration: 14s
 * tracks: 5 · items: 9
 *
 * media:
 *   name       id       type   duration  resolution  status
 *   music      m_audio  audio  30s       -           probed
 *   still      m_img    image  -         1920x1080   probed
 *   interview  m_vid    video  12.5s     3840x2160   awaiting-probe
 * cast: (none)
 * diagnostics: (none)
 * pending: 1
 * rev: 1762000005000
 */
export default (
  <timeline aspect="16:9">
    <marker id="m1" at={2} status="done" />
    <marker at={3} dur={-1} />
    <marker id="m2" />
    <track id="t_main" name="Main" kind="video">
      <clip id="i_vid" name="Interview" start={0} end={8} in={1.5} out={3} src="interview" />
    </track>
  </timeline>
);
