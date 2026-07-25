// Minimal stroke icons (Lucide-style) matching the SF Symbols milim uses.
import type { ReactNode, SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 16, children, ...rest }: P & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const Lightbulb = (p: P) => (
  <Svg {...p}><path d="M9 18h6M10 22h4M12 2a7 7 0 0 0-4 12.7c.6.5 1 1.3 1 2.1h6c0-.8.4-1.6 1-2.1A7 7 0 0 0 12 2Z" /></Svg>
);
export const FileText = (p: P) => (
  <Svg {...p}><path d="M14 3v4a1 1 0 0 0 1 1h4" /><path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2Z" /><path d="M9 9h1M9 13h6M9 17h6" /></Svg>
);
export const Code = (p: P) => (
  <Svg {...p}><path d="m9 8-4 4 4 4M15 8l4 4-4 4" /></Svg>
);
export const Terminal = (p: P) => (
  <Svg {...p}><path d="M4 17h16" /><path d="m7 7 4 4-4 4" /><path d="M13 15h4" /></Svg>
);
export const ExternalLink = (p: P) => (
  <Svg {...p}><path d="M15 3h6v6" /><path d="m10 14 11-11" /><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /></Svg>
);
export const Plug = (p: P) => (
  <Svg {...p}><path d="M9 2v6M15 2v6M7 8h10v3a5 5 0 0 1-10 0zM12 16v6" /></Svg>
);
export const Pencil = (p: P) => (
  <Svg {...p}><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></Svg>
);
export const Paperclip = (p: P) => (
  <Svg {...p}><path d="M21 11.5 12.5 20a4.5 4.5 0 0 1-6.4-6.4l8.5-8.5a3 3 0 0 1 4.3 4.3l-8.6 8.5a1.5 1.5 0 0 1-2.1-2.1l7.8-7.8" /></Svg>
);
export const Slash = (p: P) => (
  <Svg {...p}><path d="M9 20 15 4" /></Svg>
);
export const Mic = (p: P) => (
  <Svg {...p}><rect x="9" y="2" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></Svg>
);
export const UserRound = (p: P) => (
  <Svg {...p}><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></Svg>
);
export const Smartphone = (p: P) => (
  <Svg {...p}><rect x="7" y="2" width="10" height="20" rx="2" /><path d="M11 18h2" /></Svg>
);
export const Volume2 = (p: P) => (
  <Svg {...p}><path d="M11 5 6 9H3v6h3l5 4Z" /><path d="M16 8.5a5 5 0 0 1 0 7M19 5a9 9 0 0 1 0 14" /></Svg>
);
export const Shield = (p: P) => (
  <Svg {...p}><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z" /><path d="m9 12 2 2 4-5" /></Svg>
);
export const ArrowUp = (p: P) => (
  <Svg {...p}><path d="M12 19V5M6 11l6-6 6 6" /></Svg>
);
export const ArrowRight = (p: P) => (
  <Svg {...p}><path d="M5 12h14M13 6l6 6-6 6" /></Svg>
);
export const ArrowLeft = (p: P) => (
  <Svg {...p}><path d="M19 12H5M11 6l-6 6 6 6" /></Svg>
);
export const Gear = (p: P) => (
  <Svg {...p}><circle cx="12" cy="12" r="3" /><path d="M19.4 13.5a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" /></Svg>
);
export const Sliders = (p: P) => (
  <Svg {...p}><path d="M4 6h8M16 6h4M14 4v4M4 12h3M11 12h9M9 10v4M4 18h10M18 18h2M16 16v4" /></Svg>
);
export const Pin = (p: P) => (
  <Svg {...p}><path d="M9 4h6l-1 6 3 3v2H7v-2l3-3-1-6ZM12 15v5" /></Svg>
);
export const Sidebar = (p: P) => (
  <Svg {...p}><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></Svg>
);
export const ChevronDown = (p: P) => (
  <Svg {...p}><path d="m6 9 6 6 6-6" /></Svg>
);
export const Folder = (p: P) => (
  <Svg {...p}><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" /></Svg>
);
export const FolderOpen = (p: P) => (
  <Svg {...p}><path d="M3 7a2 2 0 0 1 2-2h4l2 2h4a2 2 0 0 1 2 2v1" /><path d="M3 17.5V9a2 2 0 0 1 2-2h14a2 2 0 0 1 1.9 2.6l-2.1 7A2 2 0 0 1 17 18H5a2 2 0 0 1-2-2Z" /></Svg>
);
export const Cube = (p: P) => (
  <Svg {...p}><path d="M12 2 3 7v10l9 5 9-5V7Z" /><path d="m3 7 9 5 9-5M12 12v10" /></Svg>
);
export const Sparkles = (p: P) => (
  <Svg {...p}><path d="M12 4l1.4 3.6L17 9l-3.6 1.4L12 14l-1.4-3.6L7 9l3.6-1.4ZM19 14l.7 1.8L21.5 16l-1.8.7L19 18.5l-.7-1.8L16.5 16l1.8-.5Z" /></Svg>
);
export const Star = (p: P) => (
  <Svg {...p}><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></Svg>
);
export const Image = (p: P) => (
  <Svg {...p}><rect x="3" y="5" width="18" height="14" rx="2" /><circle cx="8.5" cy="10" r="1.5" /><path d="m21 15-4.5-4.5L9 18" /></Svg>
);
export const Sun = (p: P) => (
  <Svg {...p}><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></Svg>
);
export const Check = (p: P) => (
  <Svg {...p}><path d="m5 12 5 5 9-10" /></Svg>
);
export const PlusSquare = (p: P) => (
  <Svg {...p}><rect x="3" y="3" width="18" height="18" rx="3" /><path d="M12 8v8M8 12h8" /></Svg>
);
export const Plus = (p: P) => (
  <Svg {...p}><path d="M12 5v14M5 12h14" /></Svg>
);
export const Trash = (p: P) => (
  <Svg {...p}><path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" /></Svg>
);
export const Archive = (p: P) => (
  <Svg {...p}><path d="M4 4h16v5H4Z" /><path d="M6 9v9a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V9" /><path d="M10 13h4" /></Svg>
);
export const X = (p: P) => (
  <Svg {...p}><path d="M6 6l12 12M18 6 6 18" /></Svg>
);
export const MoreHorizontal = (p: P) => (
  <Svg {...p}><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></Svg>
);
export const Square = (p: P) => (
  <Svg {...p} fill="currentColor" stroke="none"><rect x="6" y="6" width="12" height="12" rx="2.5" /></Svg>
);
export const Download = (p: P) => (
  <Svg {...p}><path d="M12 3v12M7 10l5 5 5-5M5 21h14" /></Svg>
);
export const Eye = (p: P) => (
  <Svg {...p}><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" /><circle cx="12" cy="12" r="3" /></Svg>
);
export const Globe = (p: P) => (
  <Svg {...p}><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" /></Svg>
);
export const Bolt = (p: P) => (
  <Svg {...p} fill="currentColor" stroke="none"><path d="M13 2 4 14h6l-1 8 9-12h-6l1-8Z" /></Svg>
);
export const Search = (p: P) => (
  <Svg {...p}><circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" /></Svg>
);
export const Copy = (p: P) => (
  <Svg {...p}><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h8" /></Svg>
);
export const Refresh = (p: P) => (
  <Svg {...p}><path d="M21 12a9 9 0 1 1-2.6-6.4M21 4v5h-5" /></Svg>
);
export const GitBranch = (p: P) => (
  <Svg {...p}><circle cx="6" cy="5" r="2" /><circle cx="18" cy="6" r="2" /><circle cx="6" cy="19" r="2" /><path d="M6 7v10M8 19h3a7 7 0 0 0 7-7V8" /></Svg>
);
export const GitRemote = (p: P) => (
  <Svg {...p}><path d="M7 16.5H6a3 3 0 0 1-.6-5.9A5.5 5.5 0 0 1 16 9.8a3.5 3.5 0 0 1 1 6.7h-1" /><path d="M8.5 16h7M10.5 13.8 8.3 16l2.2 2.2M13.5 13.8l2.2 2.2-2.2 2.2" /></Svg>
);
export const GitLogo = ({ size = 16, ...rest }: P) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" {...rest}>
    <path d="M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656" />
  </svg>
);
export const GitCommit = (p: P) => (
  <Svg {...p}><path d="M3 12h6M15 12h6" /><circle cx="12" cy="12" r="3" /></Svg>
);
export const GitPullRequest = (p: P) => (
  <Svg {...p}><circle cx="6" cy="6" r="2" /><circle cx="18" cy="18" r="2" /><circle cx="6" cy="18" r="2" /><path d="M6 8v8M18 16v-5a5 5 0 0 0-5-5h-1" /><path d="m14 3-3 3 3 3" /></Svg>
);
export const GitHub = ({ size = 16, ...rest }: P) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 98 96"
    fill="none"
    data-brand-logo="github"
    {...rest}
  >
    <path
      fill="currentColor"
      d="M41.4395 69.3848C28.8066 67.8535 19.9062 58.7617 19.9062 46.9902C19.9062 42.2051 21.6289 37.0371 24.5 33.5918C23.2559 30.4336 23.4473 23.7344 24.8828 20.959C28.7109 20.4805 33.8789 22.4902 36.9414 25.2656C40.5781 24.1172 44.4062 23.543 49.0957 23.543C53.7852 23.543 57.6133 24.1172 61.0586 25.1699C64.0254 22.4902 69.2891 20.4805 73.1172 20.959C74.457 23.543 74.6484 30.2422 73.4043 33.4961C76.4668 37.1328 78.0937 42.0137 78.0937 46.9902C78.0937 58.7617 69.1934 67.6621 56.3691 69.2891C59.623 71.3945 61.8242 75.9883 61.8242 81.252V91.2051C61.8242 94.0762 64.2168 95.7031 67.0879 94.5547C84.4102 87.9512 98 70.6289 98 49.1914C98 22.1074 75.9883 0 48.9043 0C21.8203 0 0 22.1074 0 49.1914C0 70.4375 13.4941 88.0469 31.6777 94.6504C34.2617 95.6074 36.75 93.8848 36.75 91.3008V83.6445C35.4102 84.2188 33.6875 84.6016 32.1562 84.6016C25.8398 84.6016 22.1074 81.1563 19.4277 74.7441C18.375 72.1602 17.2266 70.6289 15.0254 70.3418C13.877 70.2461 13.4941 69.7676 13.4941 69.1934C13.4941 68.0449 15.4082 67.1836 17.3223 67.1836C20.0977 67.1836 22.4902 68.9063 24.9785 72.4473C26.8926 75.2227 28.9023 76.4668 31.2949 76.4668C33.6875 76.4668 35.2187 75.6055 37.4199 73.4043C39.0469 71.7773 40.291 70.3418 41.4395 69.3848Z"
    />
  </svg>
);
export const Calendar = (p: P) => (
  <Svg {...p}><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16M8 14h2M14 14h2M8 17h2" /></Svg>
);
