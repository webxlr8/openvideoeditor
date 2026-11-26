# Tasks: add-ai-generation

## 1. Implementation

- [x] 1.1 Providers tested against local stand-ins (queue, cancel, auth errors).
- [x] 1.2 CLI with background worker and status; MCP tools.
- [x] 1.3 Generate panel with key entry and confirmation.
- [ ] 1.4 Verify against the live provider APIs with real keys (OpenAI request shape unverified against current docs).
- [ ] 1.5 Voice cloning, audio clean-up, stock media search. Audio clean-up done: "Clean up voice" on a clip runs RNNoise on this computer (no key, no cost); a video clip is muted and a clean copy of its sound plays alongside, an audio clip switches to the clean copy; one revision. Verified: noise before and after speech drops below a tenth, speech stays, output aligned to the sample; ops and undo tested; app e2e flow. Voice cloning and stock media search remain.
