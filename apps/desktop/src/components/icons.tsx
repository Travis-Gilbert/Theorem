// Minimal inline icon set (stroke = currentColor). No icon dependency.

import type { ReactNode } from "react";

interface IconProps {
  size?: number;
  className?: string;
}

function svg(path: ReactNode, size = 16, className?: string) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {path}
    </svg>
  );
}

export const PlusIcon = ({ size, className }: IconProps) =>
  svg(<><path d="M12 5v14" /><path d="M5 12h14" /></>, size, className);

export const GearIcon = ({ size, className }: IconProps) =>
  svg(
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </>,
    size,
    className,
  );

export const CloseIcon = ({ size, className }: IconProps) =>
  svg(<><path d="M18 6 6 18" /><path d="m6 6 12 12" /></>, size, className);

export const BackIcon = ({ size, className }: IconProps) =>
  svg(<path d="m15 18-6-6 6-6" />, size, className);

export const ForwardIcon = ({ size, className }: IconProps) =>
  svg(<path d="m9 18 6-6-6-6" />, size, className);

export const ReloadIcon = ({ size, className }: IconProps) =>
  svg(
    <>
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 3v6h-6" />
    </>,
    size,
    className,
  );

export const PinIcon = ({ size, className }: IconProps) =>
  svg(
    <path d="M9 4h6l-1 7 3 3v2H7v-2l3-3-1-7Z M12 16v4" />,
    size,
    className,
  );

export const ChatIcon = ({ size, className }: IconProps) =>
  svg(
    <path d="M21 11.5a8.38 8.38 0 0 1-8.5 8.5 8.5 8.5 0 0 1-3.8-.9L3 21l1.9-5.7A8.38 8.38 0 0 1 12.5 3 8.38 8.38 0 0 1 21 11.5Z" />,
    size,
    className,
  );

export const PanelIcon = ({ size, className }: IconProps) =>
  svg(
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M15 4v16" />
    </>,
    size,
    className,
  );

export const GlobeIcon = ({ size, className }: IconProps) =>
  svg(
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18 M12 3a15 15 0 0 1 0 18 M12 3a15 15 0 0 0 0 18" />
    </>,
    size,
    className,
  );
