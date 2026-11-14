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
const timeline = (
  <timeline aspect="16:9">
    <track id="t_titles" name="Titles" kind="video">
      <text
        id="i_text"
        name="Title"
        start={0}
        end={3}
        textContent={"Hello\nworld"}
        fontFamily="Inter"
        fontSize={96}
        fontWeight={700}
        color="#ffffff"
        textAnimation={{preset: "fade-up", speed: 1.2}}
        transform={{y: -200}}
      />
      <code
        id="i_code"
        start={3}
        end={6}
        codeComposition={{source: "export default ({time}) => <div style={{opacity: time}}>Hi</div>", width: 1920, height: 1080, assets: {logo: "m_img"}}}
      />
      <shader
        id="i_shader"
        start={6}
        end={8.25}
        shaderComposition={{template: "light-leak", params: {intensity: 0.15}, amount: 1, region: "all"}}
      />
    </track>
    <track id="t_captions" name="Captions" kind="caption">
      <caption
        id="i_cap"
        start={0.5}
        end={4}
        captionText="Hello there"
        captionSegments={[{id: "s1", text: "Hello there", startTime: 0, endTime: 1.2, wordTimings: [{word: "Hello", start: 0, end: 0.6}, {word: "there", start: 0.6, end: 1.2}]}]}
        captionPresetId="bold-pop"
        captionHighlightColor="#ffdd00"
        captionComposition={{mode: "behind-subject", fallback: "hide"}}
      />
    </track>
    <track id="t_fx" name="Adjustments" kind="adjustment">
      <adjustment
        id="i_adj"
        start={0}
        end={12.5}
        adjustmentKind="lut"
        lutPresetId="teal-orange"
        lutIntensity={0.6}
        colorGrading={{contrast: 0.1, saturation: -0.05}}
      />
    </track>
    <track id="t_main" name="Main" kind="video">
      <clip
        id="i_vid"
        name="Interview"
        start={0}
        end={8}
        in={1.5}
        out={3}
        src="interview"
        keyframeTracks={[{property: "scale", keyframes: [{time: 0, value: 1}, {time: 8, value: 1.1}], easing: "ease-in-out"}]}
        transitionOut={{type: "crossfade", duration: 0.75}}
        effects={[{id: "fx1", type: "vhs", wobble: 1}]}
        colorGrade={{lut: {preset: "film", intensity: 0.5}}}
        volume={80}
        audioFadeOut={{duration: 0.5, easing: "ease-out"}}
      />
      <image
        id="i_img"
        start={8}
        end={10}
        src="still"
        crop={{top: 10, bottom: 10}}
        opacity={0.9}
        blendMode="screen"
        borderRadius={24}
      />
      <clip
        id="i_pending"
        start={10}
        end={14}
        pending={true}
        pendingDescription="Drone shot over the harbour at dawn"
      />
    </track>
    <track id="t_music" name="Music" kind="audio" muted={true}>
      <audio id="i_music" start={0} end={14} src="music" volume={35} audioFadeIn={{duration: 1}} playbackSpeed={1.25} />
    </track>
  </timeline>
);
