import type { SocialSite } from "@/components/icons/SocialLogos";
import { SHARE_URL } from "./shareContent";

/**
 * Where a card can be posted from the dialog. No site takes an image from a
 * link, so the dialog gets the card ready and says how it goes in before
 * the site opens on a new post with the words filled in. Only sites whose
 * new post can be opened with text are here: Facebook's share link takes
 * nothing but a URL and WhatsApp's only text, so both, and Instagram, are
 * reached through the system's share sheet, which hands over the image
 * itself.
 */
export const SHARE_TARGETS: SocialSite[] = ["x", "linkedin"];

/** Brand names, the same in every language. */
export const TARGET_NAMES: Record<SocialSite, string> = {
  x: "X",
  linkedin: "LinkedIn",
};

/** Whether a site's new post takes an image pasted into it. LinkedIn's
 *  does not, so a card for it is saved as a file for its photo button. */
export const TAKES_PASTED_IMAGE: Record<SocialSite, boolean> = {
  x: true,
  linkedin: false,
};

/** The page that starts a new post on `site` with `text` and the link in
 *  it. Once the card is in, both sites drop the link's preview for it. */
export const composeUrl = (site: SocialSite, text: string): string => {
  const post = encodeURIComponent(`${text} ${SHARE_URL}`);
  switch (site) {
    case "x":
      return `https://x.com/intent/post?text=${post}`;
    case "linkedin":
      return `https://www.linkedin.com/feed/?shareActive=true&text=${post}`;
  }
};
