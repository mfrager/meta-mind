import { ButtonGrid } from "@/components/chat/button-grid.tsx";
import { ChatForm } from "@/components/chat/chat-form.tsx";
import type { ButtonRequest } from "@/lib/transcript/reducer.ts";

export function InputRequest({
  request,
  answered,
  onAnswer,
}: {
  request: ButtonRequest;
  answered?: string;
  onAnswer: (value: string) => void;
}) {
  if (request.kind === "form") {
    return (
      <ChatForm
        fields={request.fields?.length ? request.fields : [{ name: "answer", label: request.prompt ?? "Answer" }]}
        onSubmit={(values) => onAnswer(JSON.stringify(values))}
      />
    );
  }
  return <ButtonGrid request={request} answered={answered} onAnswer={onAnswer} />;
}
