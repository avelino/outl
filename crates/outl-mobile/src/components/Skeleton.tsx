import { For, JSX } from "solid-js";

/**
 * Placeholder rows shown while the workspace boots. Apple's Notes,
 * Bear, and most native iOS apps prefer skeletons over a spinner —
 * skeletons signal "content is on the way, in roughly this shape"
 * rather than "something is happening". Shimmer comes from the
 * `.outl-skeleton` CSS animation (see `styles.css`).
 *
 * Width pattern (`80%`, `60%`, …) is hand-tuned to mimic how a real
 * outline reads: a bullet + text, with varied lengths. Indent on the
 * 3rd and 4th rows hints at the tree structure the user is about to
 * see, so the load doesn't feel like a flat list pretending to be
 * something else.
 */
const SKELETON_ROWS: { indent: number; widthPercent: number }[] = [
  { indent: 0, widthPercent: 80 },
  { indent: 0, widthPercent: 60 },
  { indent: 1, widthPercent: 70 },
  { indent: 1, widthPercent: 50 },
  { indent: 0, widthPercent: 65 },
  { indent: 2, widthPercent: 45 },
];

export function SkeletonOutline(): JSX.Element {
  return (
    <div
      class="px-4 pt-6"
      aria-hidden="true"
      role="presentation"
    >
      <For each={SKELETON_ROWS}>
        {(row) => (
          <div
            class="mb-3 flex items-center gap-3"
            style={{ "padding-left": `${row.indent * 20}px` }}
          >
            {/* Bullet placeholder */}
            <span class="outl-skeleton h-1.5 w-1.5 shrink-0 rounded-full" />
            {/* Text-line placeholder */}
            <span
              class="outl-skeleton h-3 rounded-full"
              style={{ width: `${row.widthPercent}%` }}
            />
          </div>
        )}
      </For>
    </div>
  );
}
