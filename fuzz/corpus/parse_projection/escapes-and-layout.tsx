/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 1:1
 * duration: 1000000s
 * tracks: 1 · items: 3
 *
 * media:
 *   name  id  type   duration  resolution  status
 *   a     m1  video  -         -           failed
 *   a-2   m2  video  -         -           awaiting-probe
 * cast: Ana (host), Ben
 * diagnostics: (none)
 * pending: 0
 * rev: 7
 */
export default (
  <timeline aspect="1:1">
    <track id="t" name={"Quotes \"and\" more"} kind="video" locked={true} visible={false} zz={1}>
      <text id="x1" name="Ünïcödé 🎬" start={0} end={0.000001} textContent={"C:\\new\nline\t\"q\" & <tag> {brace}"} />
      <clip
        id="x2"
        start={1}
        end={2.5}
        src="a"
        transform={{x: -0.5, rotation: 359.999999, flipX: true}}
        opacity={0.333333}
        extraFields={{"z-weird key": [1, null, {"a b": true}]}}
      />
      <clip id="x3" start={2.5} end={1000000} src="a-2" volume={0} muted={true} playbackDirection="reverse" />
    </track>
  </timeline>
);
