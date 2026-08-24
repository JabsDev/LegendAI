// Camada de atalhos de teclado (tarefa 5.8). Provê um despachante genérico de
// hotkeys e o guarda de "campo de texto" — os atalhos só disparam fora de
// inputs/textarea (a não ser que o atalho não tenha `skipOnInput`).

export type HotkeyHandler = (e: KeyboardEvent) => void;

export interface Hotkey {
  key: string;
  ctrl?: boolean; // Ctrl ou Cmd (meta)
  shift?: boolean;
  alt?: boolean;
  preventDefault?: boolean;
  // quando true, não dispara se o foco estiver em um campo de texto
  skipOnInput?: boolean;
  handler: HotkeyHandler;
}

// true se o evento veio de um campo de edição de texto (input/textarea/select
// ou contenteditable). Usado para não sequestrar digitação.
export function isTextInput(e: KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null;
  if (!el) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

// Monta um handler de keydown que despacha para o primeiro atalho que casar.
// Modificadores comparados com igualdade booleana (Ctrl/Cmd tratados juntos).
export function hotkeyDispatcher(hotkeys: Hotkey[]): (e: KeyboardEvent) => void {
  return (e) => {
    const k = e.key.toLowerCase();
    const ctrl = e.ctrlKey || e.metaKey;
    for (const h of hotkeys) {
      if (h.key.toLowerCase() !== k) continue;
      if (ctrl !== !!h.ctrl) continue;
      if (e.shiftKey !== !!h.shift) continue;
      if (e.altKey !== !!h.alt) continue;
      if (h.skipOnInput && isTextInput(e)) continue;
      if (h.preventDefault) e.preventDefault();
      h.handler(e);
      return;
    }
  };
}
