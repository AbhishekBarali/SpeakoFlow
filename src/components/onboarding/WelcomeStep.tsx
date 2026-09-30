import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import Wordmark from "@/components/Wordmark";
import { Button } from "@/components/ui/Button";
import "./onboarding.css";

/** The first frame: the name, one line, one button. Same sheet as the app. */
export function WelcomeStep({ onContinue }: { onContinue: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft">
      <main className="relative flex min-h-0 flex-1 flex-col items-center justify-center overflow-hidden rounded-t-[1.25rem] border-t border-hairline bg-canvas px-6 text-center elev-pane">
        <div className="ob-welcome">
          <h1 className="ob-welcome-mark m-0 text-[3.5rem] sm:text-[4.25rem]">
            <Wordmark />
          </h1>
          <p className="ob-welcome-line mt-4 text-[1.0625rem] text-body">
            {t("onboarding.welcome.line")}
          </p>
          <div className="ob-welcome-action mt-8">
            <Button size="lg" onClick={onContinue} autoFocus>
              {t("onboarding.welcome.start")}
              <ArrowRight
                className="h-4 w-4 rtl:rotate-180"
                aria-hidden="true"
              />
            </Button>
          </div>
        </div>
        <p className="ob-welcome-note absolute bottom-6 text-[0.8125rem] text-muted">
          {t("onboarding.welcome.note")}
        </p>
      </main>
    </div>
  );
}
