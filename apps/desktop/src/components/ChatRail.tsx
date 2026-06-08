import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../state/store";
import { domainOf } from "../lib/routing";
import type { MentionRef, Tab } from "../state/types";
import { KnownContextStrip } from "./KnownContextStrip";
import { MentionPopover } from "./MentionPopover";
import { ChatIcon, CloseIcon } from "./icons";

export function ChatRail() {
  const { state, actions } = useApp();
  const active = state.tabs.find((t) => t.id === state.activeTabId);
  const activeSpace = active?.spaceId
    ? state.spaces.find((space) => space.id === active.spaceId)
    : undefined;
  const boundRoom = activeSpace?.roomId;
  const turns = active ? state.conversations[active.id]?.turns ?? [] : [];

  const domain =
    active?.kind === "web" && active.url ? domainOf(active.url) : null;
  const hits = domain ? state.recallByDomain[domain] ?? [] : [];

  const [text, setText] = useState("");
  const [mentions, setMentions] = useState<MentionRef[]>([]);
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [activeIdx, setActiveIdx] = useState(0);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const turnsRef = useRef<HTMLDivElement>(null);

  // Candidate tabs for @-mention: other open web tabs, filtered by the query.
  const candidates = useMemo<Tab[]>(() => {
    if (mentionQuery === null) return [];
    const q = mentionQuery.toLowerCase();
    return state.tabs
      .filter((t) => t.id !== state.activeTabId && t.kind === "web")
      .filter((t) => !mentions.some((m) => m.tabId === t.id))
      .filter((t) => (`${t.title} ${t.url}`).toLowerCase().includes(q))
      .slice(0, 8);
  }, [mentionQuery, state.tabs, state.activeTabId, mentions]);

  useEffect(() => {
    turnsRef.current?.scrollTo({ top: turnsRef.current.scrollHeight });
  }, [turns.length, turns[turns.length - 1]?.text]);

  const detectMention = (val: string, caret: number) => {
    const upto = val.slice(0, caret);
    const at = upto.lastIndexOf("@");
    if (at < 0) {
      setMentionQuery(null);
      return;
    }
    const between = upto.slice(at + 1);
    if (/\s/.test(between)) {
      setMentionQuery(null);
      return;
    }
    setMentionQuery(between);
    setActiveIdx(0);
  };

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setText(val);
    detectMention(val, e.target.selectionStart ?? val.length);
  };

  const pickMention = (tab: Tab) => {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? text.length;
    const upto = text.slice(0, caret);
    const at = upto.lastIndexOf("@");
    if (at >= 0) {
      setText(text.slice(0, at) + text.slice(caret));
    }
    setMentions((m) =>
      m.some((x) => x.tabId === tab.id)
        ? m
        : [...m, { tabId: tab.id, title: tab.title || tab.url, url: tab.url }],
    );
    setMentionQuery(null);
    setTimeout(() => ta?.focus(), 0);
  };

  const removeMention = (id: string) =>
    setMentions((m) => m.filter((x) => x.tabId !== id));

  const send = async () => {
    if (!text.trim() || !state.activeTabId) return;
    const t = text;
    const m = mentions;
    setText("");
    setMentions([]);
    setMentionQuery(null);
    if (state.railView === "room" && activeSpace?.id && boundRoom) {
      await actions.postRoomMessage(activeSpace.id, t);
    } else {
      await actions.sendChat(t, m);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (mentionQuery !== null && candidates.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => Math.min(i + 1, candidates.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickMention(candidates[activeIdx]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setMentionQuery(null);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  };

  return (
    <section className="rail">
      <div className="rail__head">
        <ChatIcon size={15} />
        <span className="rail__title">Chat</span>
        <span className="rail__bound">
          {state.railView === "room" && boundRoom
            ? boundRoom
            : active
              ? active.title || active.url || "New Tab"
              : "No tab"}
        </span>
        <button className="iconbtn" onClick={actions.toggleRail} title="Hide rail">
          <CloseIcon size={15} />
        </button>
      </div>

      {boundRoom && (
        <div className="rail__tabs">
          <button
            className={state.railView === "chat" ? "rail__tab rail__tab--active" : "rail__tab"}
            onClick={() => actions.setRailView("chat")}
            type="button"
          >
            Chat
          </button>
          <button
            className={state.railView === "room" ? "rail__tab rail__tab--active" : "rail__tab"}
            onClick={() => {
              actions.setRailView("room");
              if (activeSpace?.id) void actions.refreshRoom(activeSpace.id);
            }}
            type="button"
          >
            Room
          </button>
        </div>
      )}

      {state.railView === "chat" && <KnownContextStrip hits={hits} />}

      <div className="rail__turns" ref={turnsRef}>
        {state.railView === "room" && activeSpace?.id && boundRoom ? (
          <>
            <div className="participants">
              {(state.participantsBySpace[activeSpace.id] ?? []).map((participant) => (
                <span className="chip" key={participant.actor}>
                  {participant.actor}: {participant.status}
                </span>
              ))}
            </div>
            {(state.roomFeedBySpace[activeSpace.id] ?? []).map((item) => (
              <div key={item.id} className="turn turn--system">
                <div className="turn__role">{item.actor}</div>
                <div className="turn__body">{item.text}</div>
              </div>
            ))}
          </>
        ) : (
        <>
        {turns.length === 0 ? (
          <div className="rail__empty">
            Ask about this page, or @mention another tab to bring it in.
          </div>
        ) : (
          turns.map((turn) => (
            <div key={turn.id} className={"turn turn--" + turn.role}>
              <div className="turn__role">{turn.role}</div>
              {turn.mentions && turn.mentions.length > 0 && (
                <div className="turn__mentions">
                  {turn.mentions.map((m) => (
                    <span className="chip" key={m.tabId}>
                      @{m.title}
                    </span>
                  ))}
                </div>
              )}
              <div className="turn__body">
                {turn.pending ? (
                  <span className="shimmer">thinking...</span>
                ) : (
                  turn.text
                )}
              </div>
              {turn.usage && (
                <div className="turn__cost">
                  {turn.usage.model} · {turn.usage.tokensIn}/{turn.usage.tokensOut} tokens · $
                  {turn.usage.estimatedUsd.toFixed(4)}
                </div>
              )}
            </div>
          ))
        )}
        </>
        )}
      </div>

      <div className="composer">
        {mentionQuery !== null && (
          <MentionPopover
            tabs={candidates}
            activeIndex={activeIdx}
            onPick={pickMention}
          />
        )}
        {mentions.length > 0 && (
          <div className="composer__chips">
            {mentions.map((m) => (
              <span className="composer__chip" key={m.tabId}>
                @{m.title}
                <button onClick={() => removeMention(m.tabId)} title="Remove">
                  <CloseIcon size={11} />
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          ref={taRef}
          className="composer__box"
          value={text}
          onChange={onChange}
          onKeyDown={onKeyDown}
          rows={2}
          placeholder={active ? "Ask anything. Type @ to add a tab." : "Open a tab to chat."}
          disabled={!state.activeTabId}
          spellCheck
        />
        <div className="composer__hint">
          Enter to send, Shift+Enter for a newline, @ to mention a tab.
        </div>
      </div>
    </section>
  );
}
