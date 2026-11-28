// A template picked from the chat's `/` menu asks for its details first (the same fields as on Home); its request,
// with the details filled in, then goes into the chat input.

import { useState } from "react";
import { Icon } from "../icons";
import {
  type ProjectTemplate,
  type TemplateValues,
  missingFields,
  templateDefaults,
  templateFields,
  templateMessage,
} from "../templates";

export function TemplateIntake({
  template: t,
  onClose,
  onDone,
}: {
  template: ProjectTemplate;
  onClose: () => void;
  /** The request to place in the input. */
  onDone: (request: string) => void;
}) {
  const [values, setValues] = useState<TemplateValues>(() => templateDefaults(t));
  const [missing, setMissing] = useState<string[]>([]);
  const done = () => {
    const m = missingFields(t, values);
    if (m.length) return setMissing(m);
    onDone(templateMessage(t, values));
  };
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true" aria-label={t.name} data-testid="template-intake">
      <div className="modal xp-modal">
        <header>
          <h3>{t.name}</h3>
          <button className="icon-btn" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>
        <p className="muted small">{t.description}</p>
        <div className="template-fields">
          {templateFields(t).map((f, i) => {
            const bad = missing.includes(f.label);
            const common = {
              id: `ti-${f.id}`,
              value: values[f.id] ?? "",
              "aria-invalid": bad || undefined,
              "data-testid": `intake-${f.id}`,
              autoFocus: i === 0,
              onChange: (e: { target: { value: string } }) => {
                setValues((cur) => ({ ...cur, [f.id]: e.target.value }));
                setMissing([]);
              },
            };
            return (
              <label key={f.id} htmlFor={common.id}>
                <span>
                  {f.label}
                  {!f.required && <span className="muted small"> (optional)</span>}
                </span>
                {f.kind === "select" ? (
                  <select {...common}>
                    {(f.options ?? []).map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </select>
                ) : f.kind === "textarea" ? (
                  <textarea {...common} rows={2} placeholder={f.placeholder} />
                ) : (
                  <input {...common} placeholder={f.placeholder} />
                )}
              </label>
            );
          })}
        </div>
        {missing.length > 0 && (
          <p className="field-error small" role="alert" data-testid="intake-missing">
            Fill in {missing.join(", ")} first.
          </p>
        )}
        <div className="xp-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={done} data-testid="intake-done">
            Put it in the chat
          </button>
        </div>
      </div>
    </div>
  );
}
