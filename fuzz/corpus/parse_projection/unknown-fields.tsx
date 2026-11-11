/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 9:16
 * duration: 1.000001s
 * tracks: 1 · items: 1
 *
 * media:
 *   name  id  type   duration  resolution  status
 *   m1    m1  video  -         -           probed
 * cast: Ana (host)
 * diagnostics: (none)
 * pending: 0
 * rev: 2
 */
export default (
  <timeline aspect="9:16">
    <track id="t1" name="Main" kind="video" alpha={0.5} zebra="striped">
      <clip
        id="i1"
        start={0}
        end={1.000001}
        src="m1"
        transform={{x: 10, scael: 2}}
        aUnknown={3.14159}
        zUnknown={{deep: [{k: null}]}}
      />
    </track>
  </timeline>
);
