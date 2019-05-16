//---------------------------------------------------------------------------

#include <vcl.h>
#include <stdio.h>
#include <math.h>
#pragma hdrstop

#include "Unit1.h"
//---------------------------------------------------------------------------
#pragma package(smart_init)
#pragma resource "*.dfm"
TFormMain *FormMain;


const int width=12;
const int height=12;
const int levels=50;
const int tilePalette[]={0,1,3,4,5,9,13,15};
const int tilePaletteReverse[]={0,1,1,2,3,4,4,4,4,5,5,5,5,6,6,7,7,7,7};
const int rotateLeft[]={0,2,1,3,4,6,7,8,5,10,11,12,9,14,13,16,17,18,15};

#define LEFT	1
#define RIGHT	2
#define UP		4
#define DOWN	8

const int pins[]={
	0,
	LEFT|RIGHT,
	UP|DOWN,
	LEFT|RIGHT|((UP|DOWN)<<4),
	LEFT|RIGHT|UP|DOWN,
	LEFT|DOWN,
	LEFT|UP,
	UP|RIGHT,
	DOWN|RIGHT,
	LEFT|RIGHT|DOWN,
	LEFT|UP|DOWN,
	LEFT|UP|RIGHT,
	UP|RIGHT|DOWN,
	LEFT|UP|((DOWN|RIGHT)<<4),
	LEFT|DOWN|((UP|RIGHT)<<4),
	RIGHT,
	DOWN,
	LEFT,
	UP
};

unsigned char map[width*height*levels];
unsigned char levtime[levels]={248,247,247,247,239,242,243,240,246,242,242,234,243,238,241,242,243,237,236,236,227,234,222,225,211,224,224,235,232,221,233,227,212,213,178,228,221,214,210,209,229,229,236,209,207,215,196,204,188,170};

int level;
int tile;

unsigned char checked[14*14];


void __fastcall TFormMain::UpdateMap(void)
{
	int i,j,x,y;

	CheckMap();
	y=0;

	PaintBoxMap->Canvas->Brush->Style=bsClear;
	PaintBoxMap->Canvas->Pen->Color=clWhite;

	for(i=0;i<height;i++)
	{
		x=0;

		for(j=0;j<width;j++)
		{
			ImageListTiles->Draw(PaintBoxMap->Canvas,x,y,map[level*width*height+i*width+j]);
			if(checked[(i+1)*(width+2)+j+1]) PaintBoxMap->Canvas->Rectangle(x,y,x+48,y+48);
			x+=48;
		}
		y+=48;
	}

	LabelTime->Caption="Time: "+IntToStr(levtime[level]);
}



void __fastcall TFormMain::CheckMap(void)
{
	unsigned char list1[256];
	unsigned char list2[256];
	int i,j,len,newlen,dir,off,tcnt,tf,shift;
	unsigned char *list,*newlist;
	unsigned char tmap[(width+2)*(height+2)];

	for(i=0;i<(width+2)*(height+2);i++) tmap[i]=0;
	for(i=0;i<height;i++)
	{
		for(j=0;j<width;j++)
		{
			tmap[(i+1)*(width+2)+j+1]=map[level*width*height+i*width+j];
		}
	}

	list=list1;
	newlist=list2;
	len=0;
	tcnt=0;
	tf=0;

	for(i=0;i<(width+2)*(height+2);i++) checked[i]=0;

	for(i=0;i<(width+2)*(height+2);i++)
	{
		if(tmap[i]>=15&&tmap[i]<19)
		{
			list[0]=i;
			len=1;
			tcnt++;
		}
	}

	int iter=0;
	int max=0;

	while(true)
	{
		newlen=0;
		shift=0;

		for(i=0;i<len;i++)
		{
			if(list[i]==0xff)
			{
				shift=4;
				continue;
			}

			off=list[i];
			dir=pins[tmap[off]]>>shift;
			shift=0;

			if(checked[off]) if(tmap[off]>=15&&tmap[off]<19) tf++;

			if(dir&LEFT)
			{
				if(!(checked[off-1]&1))
				{
					if(pins[tmap[off-1]]&RIGHT)
					{
						newlist[newlen++]=(off-1);
						checked[off-1]|=1;
					}
				}
				if(!(checked[off-1]&2))
				{
					if(pins[tmap[off-1]]&(RIGHT<<4))
					{
						newlist[newlen++]=0xff;
						newlist[newlen++]=(off-1);
						checked[off-1]|=2;
					}
				}
			}

			if(dir&RIGHT)
			{
				if(!(checked[off+1]&1))
				{
					if(pins[tmap[off+1]]&LEFT)
					{
						newlist[newlen++]=(off+1);
						checked[off+1]|=1;
					}
				}
				if(!(checked[off+1]&2))
				{
					if(pins[tmap[off+1]]&(LEFT<<4))
					{
						newlist[newlen++]=0xff;
						newlist[newlen++]=(off+1);
						checked[off+1]|=2;
					}
				}
			}

			if(dir&UP)
			{
				if(!(checked[off-(width+2)]&1))
				{
					if(pins[tmap[off-(width+2)]]&DOWN)
					{
						newlist[newlen++]=(off-(width+2));
						checked[off-(width+2)]|=1;
					}
				}
				if(!(checked[off-(width+2)]&2))
				{
					if(pins[tmap[off-(width+2)]]&(DOWN<<4))
					{
						newlist[newlen++]=0xff;
						newlist[newlen++]=(off-(width+2));
						checked[off-(width+2)]|=2;
					}
				}
			}

			if(dir&DOWN)
			{
				if(!(checked[off+(width+2)]&1))
				{
					if(pins[tmap[off+(width+2)]]&UP)
					{
						newlist[newlen++]=(off+(width+2));
						checked[off+(width+2)]|=1;
					}
				}
				if(!(checked[off+(width+2)]&2))
				{
					if(pins[tmap[off+(width+2)]]&(UP<<4))
					{
						newlist[newlen++]=0xff;
						newlist[newlen++]=(off+(width+2));
						checked[off+(width+2)]|=2;
					}
				}
			}
		}

		if(newlen>max) max=newlen;

		if(!newlen) break;

		len=newlen;
		if(list==list1)
		{
			list=list2;
			newlist=list1;
		}
		else
		{
			list=list1;
			newlist=list2;
		}

		iter++;
		if(iter==10000) break;
	}

	LabelDone->Caption=IntToStr(iter)+" "+IntToStr(max)+"  "+IntToStr(tf)+" of "+IntToStr(tcnt);
}



//---------------------------------------------------------------------------
__fastcall TFormMain::TFormMain(TComponent* Owner)
: TForm(Owner)
{
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::FormCreate(TObject *Sender)
{
	int i;
	FILE *file;

	for(i=0;i<width*height*levels;i++) map[i]=0;

	file=fopen("levels.bin","rb");

	if(file)
	{
		fread(map,width*height*levels,1,file);
		fread(levtime,levels,1,file);
		fclose(file);

		/*int t;
		for(i=0;i<levels;i++)
		{
			t=levtime[i];
			t=255-t;
			t*=3;
			t+=i/4;
			t=t/5*5;
			if(t>250) t=250;
			levtime[i]=t;
		}*/
	}

	level=0;
	tile=0;
	LabelLevel->Caption="Level: "+IntToStr(level+1);
}
//---------------------------------------------------------------------------



void __fastcall TFormMain::PaintBoxMapPaint(TObject *Sender)
{
	UpdateMap();
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::PaintBoxPalPaint(TObject *Sender)
{
	int i;

	for(i=0;i<8;i++)
	{
		ImageListTiles->Draw(PaintBoxPal->Canvas,i*48,0,tilePalette[i]);
	}
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::PaintBoxTilePaint(TObject *Sender)
{
	ImageListTiles->Draw(PaintBoxTile->Canvas,0,0,tile);
}
//---------------------------------------------------------------------------


void __fastcall TFormMain::PaintBoxMapMouseDown(TObject *Sender,
TMouseButton Button, TShiftState Shift, int X, int Y)
{
	PaintBoxMapMouseMove(Sender,Shift,X,Y|0x1000);
}
//---------------------------------------------------------------------------


void __fastcall TFormMain::PaintBoxMapMouseMove(TObject *Sender,
TShiftState Shift, int X, int Y)
{
	int tx,ty,off,prev;
	bool down;

	down=Y&0x1000?true:false;
	Y&=~0x1000;

	if(X>=0&&X<width*48&&Y>=0&&Y<height*48)
	{
		tx=X/48;
		ty=Y/48;
		off=level*width*height+ty*width+tx;

		if(Shift.Contains(ssShift)||GetKeyState(VK_CAPITAL))
		{
			if((Shift.Contains(ssLeft)||Shift.Contains(ssRight))&&down)
			{
				map[off]=rotateLeft[map[off]];
				UpdateMap();
			}
		}
		else
		{
			if(Shift.Contains(ssLeft))
			{
				prev=map[off];
				map[off]=tile;
				if(prev!=tile) UpdateMap();
			}

			if(Shift.Contains(ssRight))
			{
				prev=tile;
				tile=map[off];
				if(prev!=tile) PaintBoxTile->Repaint();
			}
		}
	}

}
//---------------------------------------------------------------------------

void __fastcall TFormMain::PaintBoxPalMouseDown(TObject *Sender,
TMouseButton Button, TShiftState Shift, int X, int Y)
{
	PaintBoxPalMouseMove(Sender,Shift,X,Y);
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::PaintBoxPalMouseMove(TObject *Sender,
TShiftState Shift, int X, int Y)
{
	int prev;

	if(X>=0&&X<8*48&&Y>=0&&Y<48)
	{
		if(Shift.Contains(ssLeft))
		{
			prev=tile;
			tile=tilePalette[X/48];
			if(prev!=tile) PaintBoxTile->Repaint();
		}
	}
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::FormKeyDown(TObject *Sender, WORD &Key,
TShiftState Shift)
{
	bool update;
	int i,j,off,t;
	unsigned char temp[width];

	update=false;

	if(Key==VK_ADD&&level<levels-1)
	{
		level++;
		update=true;
	}

	if(Key==VK_SUBTRACT&&level>0)
	{
		level--;
		update=true;
	}

	if(Key==VK_SPACE)
	{
		for(i=0;i<width*height;i++)
		{
			for(j=0;j<rand()&7;j++) map[level*width*height+i]=rotateLeft[map[level*width*height+i]];
		}
		update=true;
	}

	if(Key==VK_LEFT)
	{
		off=level*width*height;
		for(i=0;i<height;i++) temp[i]=map[off+i*width];

		for(i=0;i<height;i++)
		{
			off=level*width*height+i*width;
			for(j=0;j<width-1;j++) map[off+j]=map[off+j+1];
		}

		off=level*width*height+width-1;
		for(i=0;i<height;i++) map[off+i*width]=temp[i];

		update=true;
	}

	if(Key==VK_RIGHT)
	{
		off=level*width*height+width-1;
		for(i=0;i<height;i++) temp[i]=map[off+i*width];

		for(i=0;i<height;i++)
		{
			off=level*width*height+i*width;
			for(j=width-1;j>0;j--) map[off+j]=map[off+j-1];
		}

		off=level*width*height;
		for(i=0;i<height;i++) map[off+i*width]=temp[i];

		update=true;
	}

	if(Key==VK_UP)
	{
		off=level*width*height;
		for(i=0;i<width;i++) temp[i]=map[off+i];

		for(i=0;i<width;i++)
		{
			off=level*width*height+i;
			for(j=0;j<height-1;j++) map[off+j*width]=map[off+(j+1)*width];
		}

		off=level*width*height+(height-1)*width;
		for(i=0;i<width;i++) map[off+i]=temp[i];

		update=true;
	}

	if(Key==VK_DOWN)
	{
		off=level*width*height+(height-1)*width;
		for(i=0;i<width;i++) temp[i]=map[off+i];

		for(i=0;i<width;i++)
		{
			off=level*width*height+i;
			for(j=height-1;j>0;j--) map[off+j*width]=map[off+(j-1)*width];
		}

		off=level*width*height;
		for(i=0;i<width;i++) map[off+i]=temp[i];

		update=true;
	}

	if(Key==VK_PRIOR)
	{
		if(level>0)
		{
			level--;
			off=level*width*height;
			for(i=0;i<width*height;i++)
			{
				j=map[off+i];
				map[off+i]=map[off+i+width*height];
				map[off+i+width*height]=j;
			}
			update=true;
			j=levtime[level];
			levtime[level]=levtime[level+1];
			levtime[level+1]=j;
		}
	}

	if(Key==VK_NEXT)
	{
		if(level<49)
		{
			off=level*width*height;
			for(i=0;i<width*height;i++)
			{
				j=map[off+i];
				map[off+i]=map[off+i+width*height];
				map[off+i+width*height]=j;
			}
			j=levtime[level];
			levtime[level]=levtime[level+1];
			levtime[level+1]=j;
			level++;
			update=true;
		}
	}

	if(Key==VK_HOME)
	{
		t=levtime[level];
		t-=5;
		if(t<0) t=0;
		levtime[level]=t;
		update=true;
	}

	if(Key==VK_END)
	{
		t=levtime[level];
		t+=5;
		if(t>255) t=255;
		levtime[level]=t;
		update=true;
	}

	if(update)
	{
		LabelLevel->Caption="Level: "+IntToStr(level+1);
		UpdateMap();
	}
}
//---------------------------------------------------------------------------

void __fastcall TFormMain::FormDestroy(TObject *Sender)
{
	FILE *file;
	int i,j,k,l,bits,off,rt;
	float tcnt;
	bool ok;

	file=fopen("levels.bin","wb");

	if(file)
	{
		fwrite(map,width*height*levels,1,file);
		fwrite(levtime,levels,1,file);
		fclose(file);
	}

	file=fopen("levels.asm","wt");

	if(file)
	{
		fprintf(file,"levList\n");

		for(i=0;i<levels;i++) fprintf(file,"\t.dw .level%i\n",i+1);

		for(i=0;i<levels;i++)
		{
			fprintf(file,".level%i\n",i+1);

			off=i*width*height;
			tcnt=0;

			for(j=0;j<width*height;j++)
			{
				if(tilePaletteReverse[map[off+j]]==7) tcnt+=1.0f;
			}

			for(j=0;j<width*height;j+=8)
			{
				bits=  tilePaletteReverse[map[off++]];
				bits|=(tilePaletteReverse[map[off++]]<<3);
				bits|=(tilePaletteReverse[map[off++]]<<6);
				bits|=(tilePaletteReverse[map[off++]]<<9);
				bits|=(tilePaletteReverse[map[off++]]<<12);
				bits|=(tilePaletteReverse[map[off++]]<<15);
				bits|=(tilePaletteReverse[map[off++]]<<18);
				bits|=(tilePaletteReverse[map[off++]]<<21);

				fprintf(file,"\t.db $%2.2x,$%2.2x,$%2.2x\n",bits&0xff,(bits>>8)&0xff,(bits>>16)&0xff);
			}

			if(!tcnt) tcnt=1;
			if(i<10) rt=0; else rt=(110-i)*10;

			fprintf(file,"\t.dw $%4.4x\n",(int)ceil(100.0f*256.0f/tcnt));
			fprintf(file,"\t.dw $%4.4x\n",rt);
			fprintf(file,"\t.db $%2.2x\n",levtime[i]);

			off=i*width*height;
			ok=false;

			for(j=1;j<height/2;j++)
			{
				for(k=height/2-j;k<height/2+j;k++)
				{
					for(l=width/2-j;l<width/2+j;l++)
					{
						if(map[off+k*width+l])
						{
							fprintf(file,"\t.db $%2.2x\n",(k+1)*16+l+1);
							ok=true;
							break;
						}
					}
					if(ok) break;
				}
				if(ok) break;
            }
		}

		fclose(file);
	}
}
//---------------------------------------------------------------------------



