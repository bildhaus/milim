import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { SheetDialog } from "./SheetDialog";
import { isComposingKeyEvent } from "../ui/shortcuts";
import { wireMessageContent } from "../api";
import {
  searchChatSessions,
  type ChatSearchResult,
  type SearchableChatSession,
} from "../lib/chatSearch";
import {
  filterCommandPaletteItems,
  type CommandPaletteItem,
} from "../lib/commandPalette";
import { sessionRecencyLabel } from "../lib/sessionRecency";
import { useSessions, type Project } from "../sessions/store";
import { searchUserChats } from "../persistence/userStateStorage.js";
import { Search, X } from "./icons";

export interface RuntimeCommand extends CommandPaletteItem {
  run: () => void;
}

const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function CommandPalette({
  projects,
  activeId,
  commands,
  onSelect,
  onClose,
}: {
  projects: Project[];
  activeId: string;
  commands: RuntimeCommand[];
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const sessions = useSessions((s) => s.sessions);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [nativeChatResults, setNativeChatResults] = useState<
    ChatSearchResult[] | null
  >(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const searchableSessions = useMemo<SearchableChatSession[]>(
    () => sessions.map((session) => ({
      id: session.id,
      title: session.title,
      updatedAt: session.updatedAt,
      archivedAt: session.archivedAt,
      settings: session.settings,
      messages: session.messages.map((message) => ({
        role: message.role,
        content: wireMessageContent(message),
      })),
    })),
    [sessions],
  );
  const commandResults = useMemo(
    () => filterCommandPaletteItems(commands, query),
    [commands, query],
  );
  const localChatResults = useMemo(
    () => searchChatSessions(searchableSessions, projects, query),
    [projects, query, searchableSessions],
  );
  useEffect(() => {
    if (!inTauri) return;
    let cancelled = false;
    setNativeChatResults(null);
    const timer = window.setTimeout(() => {
      void searchUserChats(query, 20)
        .then((results) => {
          if (!cancelled) setNativeChatResults(results);
        })
        .catch((error) => {
          if (!cancelled) {
            console.warn("Native chat search failed:", error);
            setNativeChatResults(localChatResults);
          }
        });
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [localChatResults, query]);
  const chatResults = inTauri
    ? (nativeChatResults ?? localChatResults)
    : localChatResults;
  const resultCount = commandResults.length + chatResults.length;
  const activeIndex = resultCount ? Math.min(selectedIndex, resultCount - 1) : 0;

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  function openSelected() {
    const command = commandResults[activeIndex];
    if (command) {
      onClose();
      command.run();
      return;
    }
    const chat = chatResults[activeIndex - commandResults.length];
    if (!chat) return;
    onSelect(chat.sessionId);
    onClose();
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.target !== inputRef.current || isComposingKeyEvent(event)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex((current) => resultCount ? (current + 1) % resultCount : 0);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex((current) => resultCount ? (current - 1 + resultCount) % resultCount : 0);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      openSelected();
    }
  }

  return (
    <SheetDialog title="Command palette" overlayClassName="chat-search-overlay" className="chat-search-popover" onClose={onClose}>
      <div onKeyDown={handleKeyDown}>
        <div className="chat-search-head">
          <Search size={15} />
          <input
            ref={inputRef}
            data-testid="command-palette-input"
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search commands and chats"
            aria-label="Search commands and chats"
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={resultCount ? `${listId}-${activeIndex}` : undefined}
          />
          <button className="chat-search-close" type="button" aria-label="Close command palette" onClick={onClose}>
            <X size={14} />
          </button>
        </div>

        <div id={listId} className="chat-search-list" data-testid="command-palette-results" role="listbox" aria-label="Command palette results">
          {resultCount === 0 ? (
            <div className="chat-search-empty">No commands or chats found</div>
          ) : (
            <>
              {commandResults.length > 0 && <div className="command-palette-group">Commands</div>}
              {commandResults.map((command, index) => (
                <button
                  key={command.id}
                  id={`${listId}-${index}`}
                  className={"chat-search-result command" + (index === activeIndex ? " active" : "")}
                  type="button"
                  role="option"
                  aria-selected={index === activeIndex}
                  data-testid="command-palette-command"
                  onMouseEnter={() => setSelectedIndex(index)}
                  onFocus={() => setSelectedIndex(index)}
                  onClick={() => {
                    onClose();
                    command.run();
                  }}
                >
                  <span className="chat-search-result-title">{command.label}</span>
                  {command.shortcut && <span className="chat-search-result-meta">{command.shortcut}</span>}
                </button>
              ))}

              {chatResults.length > 0 && <div className="command-palette-group">Chats</div>}
              {chatResults.map((result, index) => {
                const combinedIndex = commandResults.length + index;
                const active = combinedIndex === activeIndex;
                const metadata = [sessionRecencyLabel(result.updatedAt), result.metadata].filter(Boolean).join(" | ");
                return (
                  <button
                    key={result.sessionId}
                    id={`${listId}-${combinedIndex}`}
                    className={"chat-search-result" + (active ? " active" : "") + (result.sessionId === activeId ? " current" : "")}
                    type="button"
                    role="option"
                    aria-selected={active}
                    data-testid="command-palette-chat"
                    onMouseEnter={() => setSelectedIndex(combinedIndex)}
                    onFocus={() => setSelectedIndex(combinedIndex)}
                    onClick={() => {
                      onSelect(result.sessionId);
                      onClose();
                    }}
                  >
                    <span className="chat-search-result-title">{result.title}</span>
                    <span className="chat-search-result-meta">{metadata}</span>
                    <span className="chat-search-result-snippet">{result.snippet}</span>
                  </button>
                );
              })}
            </>
          )}
        </div>
      </div>
    </SheetDialog>
  );
}
