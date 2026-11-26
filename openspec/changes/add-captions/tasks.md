# Tasks: add-captions

## 1. Implementation

- [x] 1.1 Model download, transcription and caching; speech-model test.
- [x] 1.2 21 styles with golden checks.
- [x] 1.3 Captions panel, inspector style picker; verified in the running app.
- [ ] 1.4 Behind-subject styles (needs a subject mask).
- [x] 1.5 Assistant transcript tool and remove-silences action.
      Done: `transcript` for the built-in assistant and the pinned MCP server (`ove media transcript`, through the
      app); "Remove silences" in a clip's menu (cut at pauses, everything else follows without being chopped, one
      revision). Verified: unit tests and the app e2e flows (captions read back through the command line; a 1.5 s
      pause cut out of a clip and undone).
