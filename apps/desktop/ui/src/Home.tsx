import { useState } from "react";
import type { ProjectSummary_Serialize } from "./bindings";
import { Icon, type IconName } from "./icons";
import { formatDuration, formatWhen } from "./timecode";

const NAV: { id: string; label: string; icon: IconName }[] = [
  { id: "home", label: "Home", icon: "home" },
  { id: "projects", label: "Projects", icon: "projects" },
];
const NAV_2: { id: string; label: string; icon: IconName }[] = [{ id: "templates", label: "Templates", icon: "templates" }];
const FOOTER: { id: string; label: string; icon: IconName }[] = [
  { id: "changelog", label: "Changelog", icon: "news" },
  { id: "feedback", label: "Send feedback", icon: "flag" },
  { id: "settings", label: "Settings", icon: "settings" },
];
const CHIPS = ["Highlight reel", "Travel diary", "Interview", "Tutorial", "Product tour", "Social teaser", "Announcement"];

export function Home({
  projects,
  onOpenFile,
  onBlank,
  onOpenProject,
}: {
  projects: ProjectSummary_Serialize[];
  onOpenFile: () => void;
  onBlank: () => void;
  onOpenProject: (bundle: string) => void;
}) {
  const [picked, setTab] = useState<"templates" | "projects" | null>(null);
  const tab = picked ?? (projects.length > 0 ? "projects" : "templates");
  const [page, setPage] = useState<"home" | "projects">("home");
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
            <button key={n.id} className="nav-item">
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          {FOOTER.map((n) => (
            <button key={n.id} className="nav-item">
              <Icon name={n.icon} size={16} /> {n.label}
            </button>
          ))}
          <div className="profile">
            <span className="avatar">L</span> Local profile
          </div>
        </div>
      </aside>
      {page === "projects" ? (
        <main className="home-main projects-page" data-testid="projects-page">
          <header className="projects-head">
            <div>
              <h2>Your projects</h2>
              <p className="muted">Everything you make is saved here as you work — every change is a revision you can go back to.</p>
            </div>
            <button className="btn primary" onClick={onBlank}>
              <Icon name="plus" size={15} /> New project
            </button>
          </header>
          <ProjectGrid projects={projects} onOpen={onOpenProject} onBlank={onBlank} />
        </main>
      ) : (
      <main className="home-main">
        <section className="hero">
          <h1>What are we cutting today?</h1>
          <p className="muted">Describe the video you want, or bring footage to start.</p>
          <div className="prompt-card">
            <button className="drop-target" title="Open a media file" onClick={onOpenFile} data-testid="home-open">
              <Icon name="upload" size={18} />
            </button>
            <textarea placeholder="Describe your video, or drop footage…" rows={2} />
            <div className="prompt-row">
              <button className="icon-btn" title="Ideas (later)" disabled>
                <Icon name="spark" size={16} />
              </button>
              <span className="grow" />
              <button className="btn" onClick={onBlank} data-testid="home-blank">
                Blank project
              </button>
              <button className="send" title="Agents arrive in a later version" disabled>
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
            <button role="tab" aria-selected={tab === "templates"} className={tab === "templates" ? "on" : ""} onClick={() => setTab("templates")}>
              Templates
            </button>
            <button role="tab" aria-selected={tab === "projects"} className={tab === "projects" ? "on" : ""} onClick={() => setTab("projects")}>
              Your projects
            </button>
          </div>
          <h3>{tab === "templates" ? "Start from a template" : "Recent projects"}</h3>
          {tab === "templates" ? (
            <div className="cards">
              {TEMPLATES.map((t, i) => (
                <div key={t} className={`card card-${i}`} title="Templates arrive in a later version">
                  <span className="card-label">{t}</span>
                </div>
              ))}
            </div>
          ) : (
            <ProjectGrid projects={projects.slice(0, 8)} onOpen={onOpenProject} onBlank={onBlank} />
          )}
        </section>
      </main>
      )}
    </div>
  );
}

const TEMPLATES = ["Event recap", "Founder story", "Product launch", "Podcast clip"];

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
      <button className="project-card new" onClick={onBlank} data-testid="project-new">
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
          <div className={`project-thumb aspect-${(p.aspectRatio || "16:9").replace(":", "x")}`}>
            {p.thumbnail && <img src={p.thumbnail} alt="" />}
            <span className="badge">{p.damaged ? "damaged" : formatDuration(p.durationSeconds ?? 0)}</span>
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
