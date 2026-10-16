/** Final profile destinations, kept separate from the upstream artwork placeholders. */
export const PORTFOLIO_LINKS: Readonly<Record<number, string | null>> = Object.freeze({
  4: "https://github.com/jdharrison",
  5: "https://www.linkedin.com/in/jacobdharrison",
});

export function isPortfolioInteraction(target: number): boolean {
  return target === 4 || target === 5;
}

/** Call synchronously within the trusted canvas gesture, not after an await. */
export function openPortfolioLink(child: Pick<Window, "open">, target: number): boolean {
  const url = PORTFOLIO_LINKS[target];
  if (!isPortfolioInteraction(target) || !url) return false;
  child.open(url, "_blank", "noopener,noreferrer");
  return true;
}
