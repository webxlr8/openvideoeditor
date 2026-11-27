/*
 * ove timeline projection, format 1
 * Edit the elements below and apply with `ove source apply`. This header is informational:
 * edits to it are ignored, except `rev`, which names the revision this file was printed from.
 *
 * aspect: 16:9
 * duration: 30s
 * tracks: 1 · items: 1
 *
 * media: (none)
 * cast: (none)
 * diagnostics: (none)
 * pending: 0
 * rev: 1762000100000
 */
export default (
  <timeline aspect="16:9">
    <marker id="m_intro" at={0} />
    <marker
      id="m_tighten"
      at={12.4}
      dur={3}
      name="Tighten"
      color="#e5484d"
      status="review"
      assignee="agent"
      note="Cut the pause after the first sentence."
    />
    <marker
      id="m_done"
      at={20}
      name="Logo"
      color="blue"
      status="resolved"
      zReviewer="sam"
      extraFields={{"odd key": 1}}
    />
    <marker
      id="m_long"
      at={25.5}
      assignee="user"
      note={"This note is long on purpose so the printer has to wrap it: the opening needs a slower pace, the second shot should hold for one more beat, the logo sting is too loud against the voice, and the closing card needs the new address from the brief."}
    />
    <track id="t_main" name="Main" kind="video">
      <text id="i_title" start={0} end={30} textContent="Hi" />
    </track>
  </timeline>
);
