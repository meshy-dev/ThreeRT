// scenes/lamp-rooms.ts with 12 x 8 rooms: 768 lamps, the same view. What a sampler's noise does as the lamps multiply.
import { lampRooms } from "./lamp-rooms.ts";

export default function build() {
  return lampRooms(12, 8, "lamp-rooms-wide");
}
