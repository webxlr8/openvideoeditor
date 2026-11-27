/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 16:9
 * duration: 12s
 * tracks: 2 · items: 2
 *
 * media:
 *   name    id       type   duration  resolution  status
 *   camera  m_cam    video  20s       -           probed
 *   music   m_music  audio  30s       -           probed
 * cast: (none)
 * diagnostics: (none)
 * pending: 0
 * rev: 2
 */
export default (
  <timeline aspect="16:9">
    <track id="t_v1" name="V1" kind="video">
      <clip
        id="i_cam"
        start={0}
        end={6}
        src="camera"
        keyframeTracks={[{property: "volume", keyframes: [{time: 0, value: 100}, {time: 1.5, value: 0}, {time: 4.25, value: 200}], easing: "ease-in-out"}]}
        volume={120}
      />
    </track>
    <track id="t_music" name="Music" kind="audio">
      <audio
        id="i_music"
        start={0}
        end={12}
        src="music"
        keyframeTracks={[{property: "volume", keyframes: [{time: 2, value: 100}, {time: 2.4, value: 25.12}, {time: 5, value: 25.12}, {time: 5.4, value: 100}]}]}
      />
    </track>
  </timeline>
);
