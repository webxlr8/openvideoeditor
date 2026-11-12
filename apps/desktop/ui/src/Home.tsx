import { useState } from "react";
import type { AppError, ProjectSummary_Serialize } from "./bindings";
import { ErrorBoundary } from "./ErrorBoundary";
import { reportError } from "./notify";
import { Settings } from "./Settings";
import { PROJECT_TEMPLATES, type ProjectTemplate } from "./templates";
import { Icon, type IconName } from "./icons";
import { formatDuration, formatWhen } from "./timecode";

const NAV: { id: string; label: string; icon: IconName }[] = [
  { id: "home", label: "Home", icon: "home" },
  { id: "projects", label: "Projects", icon: "projects" },
];
const NAV_2: { id: string; label: string; icon: IconName }[] = [
  { id: "templates", label: "Templates", icon: "templates" },
];
const FOOTER: { id: string; label: string; icon: IconName }[] = [
  { id: "changelog", label: "Changelog", icon: "news" },
  { id: "feedback", label: "Send feedback", icon: "flag" },
  { id: "settings", label: "Settings", icon: "settings" },
];
const CHIPS = [
  "Highlight reel",
  "Travel diary",
  "Interview",
  "Tutorial",
  "Product tour",
  "Social teaser",
  "Announcement",
];

export function Home({
  projects,
  onOpenFile,
  onBlank,
  onOpenProject,
  onPrompt,
  onTemplate,
  onError,
}: {
  projects: ProjectSummary_Serialize[];
  onOpenFile: () => void;
  onBlank: () => void;
  onOpenProject: (bundle: string) => void;
  /** Starts a new project and hands the description to the assistant. */
  onPrompt?: (text: string) => void;
  onError?: (e: AppError) => void;
  /** Starts a project from a template with the person's details. */
  onTemplate?: (t: ProjectTemplate, details: string) => void;
}) {
  const [chosen, setChosen] = useState<ProjectTemplate | null>(null);
  const [prompt, setPrompt] = useState("");
  const submit = () => {
    const t = prompt.trim();
    if (!t || !onPrompt) return;
    setPrompt("");
    onPrompt(t);
  };
  const [picked, setTab] = useState<"templates" | "projects" | null>(null);
  const tab = picked ?? (projects.length > 0 ? "projects" : "templates");
  const [page, setPage] = useState<
    "home" | "projects" | "settings" | "templates"
  >("home");
  return (
    <div className="home" data-testid="home">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" />
          OpenVideoEditor
        </div>
        <label className="search sidebar-search">
          <Icon name="search" size={15} />
          <input placeholder="Search" />
          <kbd>⌘K</kbd>
        </label>
        <nav>
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              data-testid={`nav-${n.id}`}
              onClick={() => setPage(n.id === "projects" ? "projects" : "home")}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
          <hr />
          {NAV_2.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              data-testid={`nav-${n.id}`}
              onClick={() => setPage("templates")}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          {FOOTER.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              data-testid={`nav-${n.id}`}
              onClick={() => n.id === "settings" && setPage("settings")}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
          <div className="profile">
            <span className="avatar">L</span> Local profile
          </div>
        </div>
      </aside>
      {chosen && (
        <TemplateDialog
          template={chosen}
          onClose={() => setChosen(null)}
          onStart={(details) => {
            setChosen(null);
            onTemplate?.(chosen, details);
          }}
        />
      )}
      {page === "templates" ? (
        <main className="home-main projects-page" data-testid="templates-page">
          <header className="projects-head">
            <div>
              <h2>Templates</h2>
              <p className="muted">
                Pick a starting point, add your details, and the assistant
                builds the first cut with your media.
              </p>
            </div>
          </header>
          <TemplateGrid templates={PROJECT_TEMPLATES} onPick={setChosen} />
        </main>
      ) : page === "settings" ? (
        <main className="home-main settings-page">
          <ErrorBoundary region="Settings" className="settings">
            <Settings onError={onError ?? reportError} />
          </ErrorBoundary>
        </main>
      ) : page === "projects" ? (
        <main className="home-main projects-page" data-testid="projects-page">
          <header className="projects-head">
            <div>
              <h2>Your projects</h2>
              <p className="muted">
                Everything you make is saved here as you work — every change is
                a revision you can go back to.
              </p>
            </div>
            <button className="btn primary" onClick={onBlank}>
              <Icon name="plus" size={15} /> New project
            </button>
          </header>
          <ProjectGrid
            projects={projects}
            onOpen={onOpenProject}
            onBlank={onBlank}
          />
        </main>
      ) : (
        <main className="home-main">
          <section className="hero">
            <h1>What are we cutting today?</h1>
            <p className="muted">
              Describe the video you want, or bring footage to start.
            </p>
            <div className="prompt-card">
              <button
                className="drop-target"
                title="Open a media file"
                onClick={onOpenFile}
                data-testid="home-open"
              >
                <Icon name="upload" size={18} />
              </button>
              <textarea
                placeholder="Describe your video, or drop footage…"
                rows={2}
                value={prompt}
                data-testid="home-prompt"
                onChange={(e) => setPrompt(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    submit();
                  }
                }}
              />
              <div className="prompt-row">
                <button className="icon-btn" title="Ideas (later)" disabled>
                  <Icon name="spark" size={16} />
                </button>
                <span className="grow" />
                <button
                  className="btn"
                  onClick={onBlank}
                  data-testid="home-blank"
                >
                  Blank project
                </button>
                <button
                  className="send"
                  title="Start a project with the assistant (Enter)"
                  data-testid="home-send"
                  disabled={!prompt.trim() || !onPrompt}
                  onClick={submit}
                >
                  <Icon name="send" size={16} />
                </button>
              </div>
            </div>
            <div className="chips">
              {CHIPS.map((c) => (
                <button key={c} className="chip">
                  {c}
                </button>
              ))}
            </div>
          </section>
          <section className="shelf">
            <div className="tabs" role="tablist">
              <button
                role="tab"
                aria-selected={tab === "templates"}
                className={tab === "templates" ? "on" : ""}
                onClick={() => setTab("templates")}
              >
                Templates
              </button>
              <button
                role="tab"
                aria-selected={tab === "projects"}
                className={tab === "projects" ? "on" : ""}
                onClick={() => setTab("projects")}
              >
                Your projects
              </button>
            </div>
            <h3>
              {tab === "templates"
                ? "Start from a template"
                : "Recent projects"}
            </h3>
            {tab === "templates" ? (
              <TemplateGrid
                templates={PROJECT_TEMPLATES.slice(0, 4)}
                onPick={setChosen}
              />
            ) : (
              <ProjectGrid
                projects={projects.slice(0, 8)}
                onOpen={onOpenProject}
                onBlank={onBlank}
              />
            )}
          </section>
        </main>
      )}
    </div>
  );
}

function TemplateGrid({
  templates,
  onPick,
}: {
  templates: ProjectTemplate[];
  onPick: (t: ProjectTemplate) => void;
}) {
  return (
    <div className="cards template-cards" data-testid="template-grid">
      {templates.map((t, i) => (
        <button
          key={t.id}
          className={`card card-${i % 4} template-card`}
          onClick={() => onPick(t)}
          data-testid={`template-${t.id}`}
        >
          <span className="card-label">{t.name}</span>
          <span className="template-meta">
            {t.aspect} · {t.length}
            {t.generation ? " · AI" : ""}
          </span>
        </button>
      ))}
    </div>
  );
}

function TemplateDialog({
  template: t,
  onClose,
  onStart,
}: {
  template: ProjectTemplate;
  onClose: () => void;
  onStart: (details: string) => void;
}) {
  const [details, setDetails] = useState("");
  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={t.name}
      data-testid="template-dialog"
    >
      <div className="modal xp-modal">
        <header>
          <h3>{t.name}</h3>
          <button className="icon-btn" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>
        <p>{t.description}</p>
        <p className="muted small">
          {t.category} · {t.aspect} · {t.length}
        </p>
        <p className="small">
          <strong>Bring:</strong> {t.bring}
        </p>
        {t.generation && (
          <p className="small xp-error">
            Uses AI generation with your provider keys; the assistant tells you
            the cost before spending.
          </p>
        )}
        <textarea
          className="gen-prompt"
          rows={4}
          placeholder={t.ask}
          value={details}
          onChange={(e) => setDetails(e.target.value)}
          data-testid="template-details"
          autoFocus
        />
        <p className="muted small">
          A new project opens and the assistant starts the first cut; add your
          media to it any time.
        </p>
        <div className="xp-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn primary"
            onClick={() => onStart(details)}
            data-testid="template-start"
          >
            Start project
          </button>
        </div>
      </div>
    </div>
  );
}

function ProjectGrid({
  projects,
  onOpen,
  onBlank,
}: {
  projects: ProjectSummary_Serialize[];
  onOpen: (bundle: string) => void;
  onBlank: () => void;
}) {
  return (
    <div className="project-grid" data-testid="project-grid">
      <button
        className="project-card new"
        onClick={onBlank}
        data-testid="project-new"
      >
        <Icon name="plus" size={22} />
        <span>Blank project</span>
      </button>
      {projects.map((p) => (
        <button
          key={p.bundle}
          className={`project-card${p.damaged ? " damaged" : ""}`}
          onClick={() => !p.damaged && onOpen(p.bundle)}
          disabled={!!p.damaged}
          title={p.damaged ?? p.bundle}
          data-testid="project-card"
        >
          <div
            className={`project-thumb aspect-${(p.aspectRatio || "16:9").replace(":", "x")}`}
          >
            {p.thumbnail && <img src={p.thumbnail} alt="" />}
            <span className="badge">
              {p.damaged ? "damaged" : formatDuration(p.durationSeconds ?? 0)}
            </span>
          </div>
          <div className="project-meta">
            <strong>{p.name}</strong>
            <span className="muted small">
              {p.damaged
                ? p.damaged
                : `${p.itemCount} item${p.itemCount === 1 ? "" : "s"} · ${p.aspectRatio} · ${formatWhen(p.updatedAt ?? 0)}`}
            </span>
          </div>
        </button>
      ))}
    </div>
  );
}
