interface PieceHeatmapProps {
  progress: number;
}

export function PieceHeatmap({ progress }: PieceHeatmapProps) {
  const safeProgress = Number.isFinite(progress) ? Math.min(1, Math.max(0, progress)) : 0;
  const filled = Math.round(safeProgress * 96);
  const cells = Array.from({ length: 96 }, (_, index) => index < filled);

  return (
    <div className="heatmap" aria-label="Piece availability">
      {cells.map((cell, index) => (
        <span className={cell ? "piece filled" : "piece"} key={index} />
      ))}
    </div>
  );
}
