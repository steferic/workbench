export interface NavItem {
  label: string;
  /** Semantic status: drives the leading dot and screen-reader text. */
  status: "active" | "unread" | "idle";
  badge?: string;
}

export interface NavSection {
  label: string;
  items: NavItem[];
}

export const NAV_SECTIONS: NavSection[] = [
  {
    label: "fluid-functionalism",
    items: [
    { label: "New pricing page exploration", status: "active" },
    { label: "Component library audit", status: "idle" },
    { label: "Dark mode token pass", status: "unread" },
    ],
  },
  {
    label: "portfolio-site",
    items: [
    { label: "Scrollbar fade regression", status: "idle" },
    { label: "Registry deploy pipeline", status: "unread" },
    ],
  },
];
