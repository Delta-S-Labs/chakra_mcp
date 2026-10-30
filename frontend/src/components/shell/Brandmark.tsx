import Image from "next/image";

export default function Brandmark() {
  return (
    <div className="brand-lockup">
      <div className="brand-mark">
        <Image
          src="/brand/mark.svg"
          alt="ChakraMCP logo"
          width={28}
          height={28}
          className="brand-mark-icon"
          priority
        />
        ChakraMCP
      </div>
    </div>
  );
}
