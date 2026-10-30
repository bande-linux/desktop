import math
def oklch(L,C,h):
    a=C*math.cos(math.radians(h)); b=C*math.sin(math.radians(h))
    l_=L+0.3963377774*a+0.2158037573*b; m_=L-0.1055613458*a-0.0638541728*b; s_=L-0.0894841775*a-1.2914855480*b
    l,m,s=l_**3,m_**3,s_**3
    r=4.0767416621*l-3.3077115913*m+0.2309699292*s
    g=-1.2684380046*l+2.6097574011*m-0.3413193965*s
    bb=-0.0041960863*l-0.7034186147*m+1.7076147010*s
    def enc(x):
        x=min(max(x,0),1); return 12.92*x if x<=0.0031308 else 1.055*x**(1/2.4)-0.055
    return '#%02X%02X%02X'%tuple(round(enc(v)*255) for v in (r,g,bb))
def lin(c):
    c=c/255; return c/12.92 if c<=0.04045 else ((c+0.055)/1.055)**2.4
def rgb(h): h=h.lstrip('#'); return [int(h[i:i+2],16) for i in (0,2,4)]
def lum(h): r,g,b=map(lin,rgb(h)); return 0.2126*r+0.7152*g+0.0722*b
def wcag(a,b):
    x,y=sorted([lum(a),lum(b)],reverse=True); return (x+0.05)/(y+0.05)
def apca(txt,bg):
    def Y(h):
        r,g,b=[(c/255)**2.4 for c in rgb(h)]; y=0.2126729*r+0.7151522*g+0.0721750*b
        return y if y>0.022 else y+(0.022-y)**1.414
    yt,yb=Y(txt),Y(bg)
    if yb>yt: s=(yb**0.56-yt**0.57)*1.14; return 0 if s<0.1 else (s-0.027)*100
    s=(yb**0.65-yt**0.62)*1.14; return 0 if s>-0.1 else (s+0.027)*100
H=80
T={}
light=dict(chassis=(0.938,0.005),sheet=(0.988,0.0025),overlay=(1.0,0),well=(0.905,0.007),
 ink=(0.235,0.008),ink2=(0.46,0.010),ink3=(0.60,0.010),hairline=(0.86,0.008),edge=(0.78,0.008))
dark=dict(chassis=(0.185,0.006),sheet=(0.225,0.006),overlay=(0.265,0.006),well=(0.275,0.006),
 ink=(0.955,0.004),ink2=(0.78,0.008),ink3=(0.64,0.008),hairline=(0.31,0.006),edge=(0.38,0.006))
for name,sch in (('light',light),('dark',dark)):
    for k,(L,C) in sch.items(): T[name+'.'+k]=oklch(L,C,H)
acc={'ultramarine':(0.53,0.20,272,0.70,0.15),'teal':(0.55,0.10,195,0.74,0.11),'moss':(0.55,0.12,140,0.75,0.13),
 'amber':(0.62,0.14,65,0.80,0.13),'ember':(0.58,0.19,38,0.72,0.16),'rose':(0.56,0.19,5,0.73,0.15),
 'plum':(0.52,0.16,320,0.72,0.13),'graphite':(0.30,0.01,H,0.90,0.01)}
for k,(L,C,h,L2,C2) in acc.items(): T['accent.light.'+k]=oklch(L,C,h); T['accent.dark.'+k]=oklch(L2,C2,h)
sem={'danger':(0.57,0.20,25,0.70,0.17),'warning':(0.70,0.15,70,0.80,0.14),'success':(0.56,0.13,150,0.74,0.14)}
for k,(L,C,h,L2,C2) in sem.items(): T['sem.light.'+k]=oklch(L,C,h); T['sem.dark.'+k]=oklch(L2,C2,h)
if __name__=='__main__':
    for k,v in T.items(): print(k,v)
    for s in ('light','dark'):
        for bg in ('chassis','sheet','overlay'):
            for fg in ('ink','ink2','ink3'):
                a,b=T[f'{s}.{fg}'],T[f'{s}.{bg}']
                print(f'{s} {fg} on {bg}: wcag {wcag(a,b):.2f} apca {apca(a,b):.0f}')
        print(s,'edge vs sheet',round(wcag(T[s+'.edge'],T[s+'.sheet']),2))
    for k in acc:
        for s,bgk in (('light','sheet'),('dark','sheet')):
            c=T[f'accent.{s}.{k}']
            fgw = '#FFFFFF'; fgd=T['light.ink']
            print(k,s,c,'white on',round(wcag('#FFFFFF',c),2),'ink on',round(wcag(T['light.ink'],c),2),'vs sheet',round(wcag(c,T[s+'.'+bgk]),2))
