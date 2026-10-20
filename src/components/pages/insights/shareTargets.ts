import type { SocialSite } from "@/components/icons/SocialLogos";
import { SHARE_URL } from "./shareContent";

/**
 * Where a card can be posted in one click. No site takes an image from a
 * link, so the card goes on the clipboard first and the site opens on a new
 * post with the text filled in, ready for the image to be pasted. Instagram
 * has no new-post link at all and is reached through the system's share
 * sheet instead, by way of the phone.
 */
export const SHARE_TARGETS: SocialSite[] = [
  "x",
  "linkedin",
  "facebook",
  "whatsapp",
];

/** Brand names, the same in every language. */
export const TARGET_NAMES: Record<SocialSite, string> = {
  x: "X",
  linkedin: "LinkedIn",
  facebook: "Facebook",
  threads: "Threads",
  whatsapp: "WhatsApp",
};

/**
 * The page that starts a new post on `site` with `text` and the link in it.
 * Facebook fills nothing in from a link, so it opens on the feed, whose own
 * box takes the pasted image.
 */
export const composeUrl = (site: SocialSite, text: string): string => {
  const post = encodeURIComponent(`${text} ${SHARE_URL}`);
  switch (site) {
    case "x":
      return `https://x.com/intent/post?text=${post}`;
    case "linkedin":
      return `https://www.linkedin.com/feed/?shareActive=true&text=${post}`;
    case "facebook":
      return "https://www.facebook.com/";
    case "threads":
      return `https://www.threads.com/intent/post?text=${post}`;
    case "whatsapp":
      return `https://wa.me/?text=${post}`;
  }
};
