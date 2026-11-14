import { useEffect, useRef, useState } from "react";
import type { AppError, ProjectSummary_Serialize } from "./bindings";
import { ErrorBoundary } from "./ErrorBoundary";
import { reportError } from "./notify";
import { type HelpActions, HelpMenu } from "./onboarding/HelpMenu";
import { Settings, type SettingsSection } from "./Settings";
import { FeedbackDialog, WhatsNew } from "./Support";
import { MOD } from "./keys";
import {
  PROJECT_TEMPLATES,
  type ProjectTemplate,
  type TemplateValues,
  missingFields,
  templateDefaults,
  templateFields,
} from "./templates";
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
/** Starting points for the prompt: a chip fills it in for the person to finish (nothing starts until they send). */
export const CHIPS: { label: string; prompt: string }[] = [
  { label: "Highlight reel", prompt: "Make a highlight reel of my footage: " },
  { label: "Travel diary", prompt: "Make a travel diary from my clips: " },
  { label: "Interview", prompt: "Cut an interview into a short video: " },
  { label: "Tutorial", prompt: "Make a step-by-step tutorial: " },
  { label: "Product tour", prompt: "Make a product tour: " },
  { label: "Social teaser", prompt: "Make a vertical social teaser: " },
  { label: "Announcement", prompt: "Make a short announcement video: " },
];

/** Projects whose name contains the search words. */
export function matchProjects(
  projects: ProjectSummary_Serialize[],
  query: string,
): ProjectSummary_Serialize[] {
  const q = query.trim().toLowerCase();
  return q ? projects.filter((p) => p.name.toLowerCase().includes(q)) : projects;
}

export function Home({
  projects,
  onOpenFile,
  onBlank,
  onOpenProject,
  onPrompt,
  onTemplate,
  onError,
  onSample,
  help,
}: {
  projects: ProjectSummary_Serialize[];
  onOpenFile: () => void;
  onBlank: () => void;
  /** Opens a new copy of the sample project. */
  onSample?: () => void;
  /** What the Help entry reopens (the welcome, the checklist, the sample). */
  help?: HelpActions;
  onOpenProject: (bundle: string) => void;
  /** Starts a new project and hands the description to the assistant. */
  onPrompt?: (text: string) => void;
  onError?: (e: AppError) => void;
  /** Starts a project from a template with what the person filled in. */
  onTemplate?: (t: ProjectTemplate, values: TemplateValues) => void;
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
  const [dialog, setDialog] = useState<"changelog" | "feedback" | null>(null);
  /** The Settings section to open on (Help → Keyboard shortcuts goes straight to Shortcuts), and how many times
   * Settings was asked for: asking again shows that section even when Settings is already open on another. */
  const [settings, setSettings] = useState<{ section: SettingsSection; asked: number }>({
    section: "library",
    asked: 0,
  });
  const openSettings = (section: SettingsSection) => {
    setSettings((s) => ({ section, asked: s.asked + 1 }));
    setPage("settings");
  };
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  // ⌘K / Ctrl+K: search projects.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const search = (q: string) => {
    setQuery(q);
    if (q.trim()) setPage("projects");
  };
  const found = matchProjects(projects, query);
  return (
    <div className="home" data-testid="home">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" />
          OpenVideoEditor
        </div>
        <label className="search sidebar-search">
          <Icon name="search" size={15} />
          <input
            ref={searchRef}
            placeholder="Search projects"
            aria-label="Search projects"
            data-testid="home-search"
            value={query}
            onChange={(e) => search(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") search("");
            }}
          />
          <kbd>{MOD === "⌘" ? "⌘K" : "Ctrl K"}</kbd>
        </label>
        <nav>
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              aria-current={n.id === page ? "page" : undefined}
              data-testid={`nav-${n.id}`}
              onClick={() => {
                setQuery("");
                setPage(n.id === "projects" ? "projects" : "home");
              }}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
          <hr />
          {NAV_2.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              aria-current={n.id === page ? "page" : undefined}
              data-testid={`nav-${n.id}`}
              onClick={() => setPage("templates")}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <HelpMenu
            variant="nav"
            actions={help}
            onShortcuts={() => openSettings("shortcuts")}
          />
          {FOOTER.map((n) => (
            <button
              key={n.id}
              className={`nav-item${n.id === page ? " active" : ""}`}
              aria-current={n.id === page ? "page" : undefined}
              data-testid={`nav-${n.id}`}
              onClick={() => {
                if (n.id === "settings") openSettings("library");
                else setDialog(n.id === "changelog" ? "changelog" : "feedback");
              }}
            >
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
          <div
            className="profile"
            title="No account: your projects, settings and keys stay on this computer."
          >
            <span className="avatar">L</span> Local profile
          </div>
        </div>
      </aside>
      {dialog === "changelog" && <WhatsNew onClose={() => setDialog(null)} />}
      {dialog === "feedback" && (
        <FeedbackDialog
          onClose={() => setDialog(null)}
          onError={onError ?? reportError}
        />
      )}
      {chosen && (
        <TemplateDialog
          template={chosen}
          onClose={() => setChosen(null)}
          onStart={(values) => {
            setChosen(null);
            onTemplate?.(chosen, values);
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
            <Settings
              key={settings.asked}
              initialSection={settings.section}
              onError={onError ?? reportError}
            />
          </ErrorBoundary>
        </main>
      ) : page === "projects" ? (
        <main className="home-main projects-page" data-testid="projects-page">
          <header className="projects-head">
            <div>
              <h2>
                {query.trim()
                  ? `Projects matching “${query.trim()}”`
                  : "Your projects"}
              </h2>
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
            projects={found}
            onOpen={onOpenProject}
            onBlank={onBlank}
          />
          {query.trim() && found.length === 0 && (
            <p className="muted" data-testid="search-empty">
              No projects match “{query.trim()}”.
            </p>
          )}
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
                ref={promptRef}
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
                <span className="grow" />
                {onSample && (
                  <button
                    className="btn"
                    title="Open a short sample project to try things on"
                    onClick={onSample}
                    data-testid="home-sample"
                  >
                    Try the sample
                  </button>
                )}
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
                <button
                  key={c.label}
                  className="chip"
                  title="Start the description; add your details, then send"
                  data-testid={`chip-${c.label}`}
                  onClick={() => {
                    setPrompt(c.prompt);
                    const el = promptRef.current;
                    if (!el) return;
                    el.focus();
                    // The caret goes after the starter words once React has put them in.
                    requestAnimationFrame(() =>
                      el.setSelectionRange(c.prompt.length, c.prompt.length),
                    );
                  }}
                >
                  {c.label}
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
  onStart: (values: TemplateValues) => void;
}) {
  const [values, setValues] = useState<TemplateValues>(() =>
    templateDefaults(t),
  );
  const [missing, setMissing] = useState<string[]>([]);
  const set = (id: string, v: string) => {
    setValues((cur) => ({ ...cur, [id]: v }));
    setMissing([]);
  };
  const start = () => {
    const m = missingFields(t, values);
    if (m.length) return setMissing(m);
    onStart(values);
  };
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
        <div className="template-fields" data-testid="template-fields">
          {templateFields(t).map((f, i) => {
            const bad = missing.includes(f.label);
            const common = {
              id: `tf-${f.id}`,
              className: bad ? "invalid" : undefined,
              value: values[f.id] ?? "",
              "aria-invalid": bad || undefined,
              "aria-required": f.required || undefined,
              "data-testid":
                f.id === "details" ? "template-details" : `template-field-${f.id}`,
              autoFocus: i === 0,
            };
            return (
              <label key={f.id} htmlFor={common.id}>
                <span>
                  {f.label}
                  {f.required ? (
                    <span className="field-error" aria-hidden>
                      {" "}
                      *
                    </span>
                  ) : (
                    <span className="muted small"> (optional)</span>
                  )}
                </span>
                {f.kind === "select" ? (
                  <select
                    {...common}
                    onChange={(e) => set(f.id, e.target.value)}
                  >
                    {(f.options ?? []).map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </select>
                ) : f.kind === "textarea" ? (
                  <textarea
                    {...common}
                    rows={f.id === "details" || f.id === "media" ? 2 : 3}
                    placeholder={f.placeholder}
                    onChange={(e) => set(f.id, e.target.value)}
                  />
                ) : (
                  <input
                    {...common}
                    placeholder={f.placeholder}
                    onChange={(e) => set(f.id, e.target.value)}
                  />
                )}
              </label>
            );
          })}
        </div>
        {missing.length > 0 && (
          <p className="field-error small" role="alert" data-testid="template-missing">
            Fill in {missing.join(", ")} to start.
          </p>
        )}
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
            onClick={start}
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
