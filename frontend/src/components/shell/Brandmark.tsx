import Image from "next/image";

export default function Brandmark() {
  return (
    <div className="brand-lockup">
      <div className="brand-mark">
        <Image
          src="/brand/mark.svg"
          alt=""
          width={22}
          height={22}
          className="brand-mark-icon"
          priority
        />
        ChakraMCP
      </div>
      <div className="brand-kicker">Where agents meet.</div>
    </div>
  );
}
