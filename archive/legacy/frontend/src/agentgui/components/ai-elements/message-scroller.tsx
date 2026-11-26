"use client";

import { Button } from "@/agentgui/components/ui/button";
import { cn } from "@/agentgui/lib/utils";
import { ArrowDownIcon } from "lucide-react";
import type { ComponentProps } from "react";
import { StickToBottom, useStickToBottomContext } from "use-stick-to-bottom";

export type MessageScrollerProps = ComponentProps<typeof StickToBottom>;

/** Scroll container that keeps the latest message pinned while streaming. */
export const MessageScroller = ({
  className,
  ...props
}: MessageScrollerProps) => (
  <StickToBottom
    className={cn("relative flex-1 overflow-y-hidden", className)}
    initial="smooth"
    resize="smooth"
    role="log"
    {...props}
  />
);

export type MessageScrollerContentProps = ComponentProps<
  typeof StickToBottom.Content
>;

export const MessageScrollerContent = ({
  className,
  ...props
}: MessageScrollerContentProps) => (
  <StickToBottom.Content
    className={cn("flex flex-col gap-6 px-5 py-6", className)}
    {...props}
  />
);

export const MessageScrollerButton = () => {
  const { isAtBottom, scrollToBottom } = useStickToBottomContext();

  if (isAtBottom) return null;

  return (
    <Button
      className="absolute bottom-3 left-1/2 z-10 -translate-x-1/2 rounded-full border-border/80 bg-surface/90 shadow-popover backdrop-blur"
      onClick={() => scrollToBottom()}
      size="sm"
      type="button"
      variant="outline"
    >
      <ArrowDownIcon className="size-3.5" />
      Jump to latest
    </Button>
  );
};
