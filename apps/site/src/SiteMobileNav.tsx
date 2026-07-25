import { useEffect, useRef, useState } from "react";

export type SiteNavLink = {
  label: string;
  href: string;
  className?: string;
};

export function SiteMobileNav({ links }: { links: readonly SiteNavLink[] }) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        toggleRef.current?.focus();
        return;
      }
      if (event.key !== "Tab") return;

      const links = Array.from(menuRef.current?.querySelectorAll<HTMLElement>("a") ?? []);
      const first = links[0];
      const last = links[links.length - 1];
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    menuRef.current?.querySelector<HTMLElement>("a")?.focus();
    window.addEventListener("keydown", onKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <>
      <button
        ref={toggleRef}
        className="menu-toggle"
        type="button"
        aria-controls="mobile-nav"
        aria-expanded={open}
        aria-label={open ? "Close navigation" : "Open navigation"}
        data-open={open ? "" : undefined}
        onClick={() => setOpen((current) => !current)}
      >
        <span className="menu-toggle-icon menu-toggle-icon-menu"><MenuIcon /></span>
        <span className="menu-toggle-icon menu-toggle-icon-close"><CloseIcon /></span>
      </button>
      <nav ref={menuRef} className="mobile-menu" id="mobile-nav" aria-label="Mobile primary" hidden={!open}>
        {links.map((link) => (
          <a className={link.className} href={link.href} key={link.href} onClick={() => setOpen(false)}>
            {link.label}
          </a>
        ))}
      </nav>
    </>
  );
}

function MenuIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M4 7h16M4 12h16M4 17h16" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  );
}
