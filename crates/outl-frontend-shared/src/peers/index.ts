/**
 * Pairing + peer primitives shared by every GUI client.
 *
 * Components ({@link PairingQR}, {@link PeerList}) are pure / stateless —
 * data + callbacks flow through props, no Tauri command runs inside.
 * The typed `peer*` command wrappers live in `@outl/shared/api/commands`;
 * the `PeerDto` / `PeerStatusDto` DTOs in `@outl/shared/api/types`.
 *
 * Import the baseline chrome with `@import "@outl/shared/peers/styles"`.
 */

export { PairingQR } from "./PairingQR";
export { PeerList } from "./PeerList";
export { ticketToSvg, type TicketQrOptions } from "./qr";
export { peersOnline } from "./status";
