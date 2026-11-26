export function Table({ caption }: { caption: string }) {
  return (
    <table data-token="table" className="w-full border-collapse text-left">
      <caption className="micro-label mb-1 text-left">{caption}</caption>
      <thead>
        <tr>
          <th className="micro-label border-border border-b p-1">Service</th>
          <th className="micro-label border-border border-b p-1">State</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td className="p-1 font-mono text-value">sqlite</td>
          <td className="p-1 text-small">ok</td>
        </tr>
      </tbody>
    </table>
  );
}
