import { useState } from "react";
import { Icon, type IconName } from "./icons";

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

export function Home({ onOpenFile, onBlank }: { onOpenFile: () => void; onBlank: () => void }) {
  const [tab, setTab] = useState<"templates" | "projects">("templates");
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
            <button key={n.id} className={`nav-item${n.id === "home" ? " active" : ""}`}>
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
          <div className="cards">
            {(tab === "templates" ? [0, 1, 2, 3] : []).map((i) => (
              <div key={i} className={`card card-${i}`} />
            ))}
            {tab === "projects" && <p className="muted">Projects arrive with the project library (change 2).</p>}
          </div>
        </section>
      </main>
    </div>
  );
}
