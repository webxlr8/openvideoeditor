# Cloud API (client-observed)

Base `/api/v1`, bearer tokens from OAuth (browser flow for the app, device flow for the CLI). Shapes are those the
client sends and parses; server internals were not observed.

- **agent-proxy**: `/<agent-proxy>/v1/messages`
- **agent-turns**: `/api/v1/agent-turns`, `/api/v1/agent-turns/:p`
- **audio-processing**: `/api/v1/audio-processing`
- **auth**: `/api/v1/auth/config`
- **card**: `/api/v1/card`, `/api/v1/card/share`, `/api/v1/card/signature`
- **changelog**: `/api/v1/changelog/latest`, `/api/v1/changelog/seen`
- **cloud-projects**: `/api/v1/cloud-projects`, `/api/v1/cloud-projects/:p`
- **cloud-time**: `/api/v1/cloud-time`
- **desktop**: `/api/v1/desktop/access`, `/api/v1/desktop/activation`, `/api/v1/desktop/telemetry`, `/api/v1/desktop/update`
- **exports**: `/api/v1/exports/:p`, `/api/v1/exports/allowance`, `/api/v1/exports/claim`, `/api/v1/exports/cloud`
- **feedback**: `/api/v1/feedback`, `/api/v1/feedback/uploads`
- **footage**: `/api/v1/footage/search`
- **generations**: `/api/v1/generations`, `/api/v1/generations/:p/cancel`, `/api/v1/generations/:p/retry-storage`, `/api/v1/generations/by-key/:p`, `/api/v1/generations/models`
- **mattes**: `/api/v1/mattes`
- **me**: `/api/v1/me`
- **onboarding**: `/api/v1/onboarding`
- **plans**: `/api/v1/plans`
- **plugin**: `/api/v1/plugin/previews/report`
- **prompts**: `/api/v1/prompts/improve`
- **recipes**: `/api/v1/recipes`, `/api/v1/recipes/announcement`, `/api/v1/recipes/search`, `/api/v1/recipes/search/account`
- **sounds**: `/api/v1/sounds`
- **transcriptions**: `/api/v1/transcriptions`
- **uploads**: `/api/v1/uploads/:p/proxy`
- **voiceovers**: `/api/v1/voiceovers`
- **voices**: `/api/v1/voices`
- **youtube-imports**: `/api/v1/youtube-imports`

Total: 45 endpoints. Hosted-agent LLM traffic goes through an Anthropic-Messages-compatible proxy path that meters usage per plan.
